-- Ambiance et régulation de ventilation (EO_Sevel).
--
-- Ce schéma est volontairement séparé du reste : le démon de régulation
-- (binaire eo-regul) ne partage aucun code avec l'application, il communique
-- avec elle uniquement par ces tables. L'application écrit les consignes, le
-- démon écrit les mesures et l'état. Chacun peut redémarrer sans l'autre.

-- Courbes de consigne suivant l'âge des animaux. C'est ce qui distingue une
-- régulation branchée sur un logiciel d'élevage d'un simple thermostat : la
-- consigne descend toute seule au fil des jours de présence de la bande.
CREATE TABLE IF NOT EXISTS ambiance_courbe (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nom TEXT NOT NULL,
    stade TEXT,
    note TEXT
);

-- Un point = une consigne à un jour de présence donné. Entre deux points la
-- consigne est interpolée linéairement ; avant le premier et après le dernier
-- elle est tenue à la valeur du point extrême (jamais extrapolée).
CREATE TABLE IF NOT EXISTS ambiance_courbe_point (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    courbe_id INTEGER NOT NULL REFERENCES ambiance_courbe(id),
    jour INTEGER NOT NULL,
    t_cible REAL NOT NULL,
    ventil_min REAL,
    ventil_max REAL,
    UNIQUE(courbe_id, jour)
);

-- Configuration de régulation d'une salle. Une salle sans ligne ici n'est pas
-- régulée du tout : l'ajout d'une salle dans Structure ne met jamais en route
-- une ventilation toute seule.
CREATE TABLE IF NOT EXISTS ambiance_salle (
    salle_id INTEGER PRIMARY KEY REFERENCES salle(id),
    actif INTEGER NOT NULL DEFAULT 0,
    pilote INTEGER NOT NULL DEFAULT 0,

    -- Adressage IPX800 V5. Les identifiants sont ceux des objets tels que
    -- l'IPX les expose dans son API ; ils restent du texte pour ne pas
    -- dépendre d'un format de numérotation qui changerait avec le firmware.
    ipx_hote TEXT,
    entree_t TEXT,
    entree_h TEXT,
    entree_lux TEXT,
    sortie_ventil TEXT,

    -- Consigne : soit une courbe liée à l'âge de la bande, soit une valeur
    -- fixe si aucune courbe n'est choisie.
    courbe_id INTEGER REFERENCES ambiance_courbe(id),
    t_cible_fixe REAL NOT NULL DEFAULT 22.0,
    plage REAL NOT NULL DEFAULT 3.0,
    ventil_min REAL NOT NULL DEFAULT 20.0,
    ventil_max REAL NOT NULL DEFAULT 100.0,
    hygro_cible REAL,
    hygro_influence REAL NOT NULL DEFAULT 0.0,

    -- Anticipation : part du gradient de température (°C/h) ajoutée à l'écart
    -- avant calcul. 0 = régulation purement réactive.
    anticipation REAL NOT NULL DEFAULT 0.0,

    -- Garde-fous. Une mesure hors de ces bornes, ou plus vieille que
    -- fraicheur_max_s, n'est jamais utilisée pour réguler : la salle passe en
    -- repli de sécurité et une alerte est ouverte.
    t_plausible_min REAL NOT NULL DEFAULT 2.0,
    t_plausible_max REAL NOT NULL DEFAULT 45.0,
    h_plausible_min REAL NOT NULL DEFAULT 5.0,
    h_plausible_max REAL NOT NULL DEFAULT 99.0,
    fraicheur_max_s INTEGER NOT NULL DEFAULT 1800,
    ventil_securite REAL NOT NULL DEFAULT 60.0,

    -- Seuils d'alerte d'ambiance, indépendants de la régulation.
    t_alerte_min REAL,
    t_alerte_max REAL,
    h_alerte_max REAL,

    -- Bande morte d'écriture : on n'écrit sur l'IPX que si la consigne bouge
    -- d'au moins ce pourcentage, ou au bout de relance_s secondes.
    deadband_pct REAL NOT NULL DEFAULT 2.0,
    relance_s INTEGER NOT NULL DEFAULT 300,

    note TEXT
);

-- Mesures brutes. Valeurs en dixièmes d'unité dans un entier : une
-- température au dixième de degré tient dans deux octets utiles au lieu de
-- huit, et l'index composite évite une colonne d'identifiant inutile.
-- grandeur : 't' | 'h' | 'lux' | 'ventil' | 'consigne'
CREATE TABLE IF NOT EXISTS ambiance_mesure (
    salle_id INTEGER NOT NULL,
    grandeur TEXT NOT NULL,
    ts INTEGER NOT NULL,
    valeur INTEGER NOT NULL,
    bande_id INTEGER,
    PRIMARY KEY (salle_id, grandeur, ts)
) WITHOUT ROWID;

-- Agrégats horaires, conservés bien plus longtemps que le brut.
CREATE TABLE IF NOT EXISTS ambiance_heure (
    salle_id INTEGER NOT NULL,
    grandeur TEXT NOT NULL,
    heure INTEGER NOT NULL,
    moy INTEGER NOT NULL,
    mini INTEGER NOT NULL,
    maxi INTEGER NOT NULL,
    n INTEGER NOT NULL,
    bande_id INTEGER,
    PRIMARY KEY (salle_id, grandeur, heure)
) WITHOUT ROWID;

-- Agrégats journaliers, conservés sans limite : c'est cette table qui sert
-- aux croisements ambiance / performances sur plusieurs bandes.
CREATE TABLE IF NOT EXISTS ambiance_jour (
    salle_id INTEGER NOT NULL,
    grandeur TEXT NOT NULL,
    jour TEXT NOT NULL,
    moy INTEGER NOT NULL,
    mini INTEGER NOT NULL,
    maxi INTEGER NOT NULL,
    n INTEGER NOT NULL,
    minutes_hors_plage INTEGER NOT NULL DEFAULT 0,
    bande_id INTEGER,
    PRIMARY KEY (salle_id, grandeur, jour)
) WITHOUT ROWID;

-- Dernier état connu de chaque salle régulée. Une seule ligne par salle,
-- réécrite à chaque cycle : c'est ce que lit l'application pour l'affichage,
-- sans jamais interroger l'IPX elle-même.
CREATE TABLE IF NOT EXISTS ambiance_etat (
    salle_id INTEGER PRIMARY KEY REFERENCES salle(id),
    ts INTEGER NOT NULL,
    t REAL,
    h REAL,
    lux REAL,
    consigne REAL,
    ventil REAL,
    bande_id INTEGER,
    jour_bande INTEGER,
    mode TEXT NOT NULL DEFAULT 'auto',
    sonde_ok INTEGER NOT NULL DEFAULT 1,
    message TEXT
);

-- Battement de cœur du démon. L'application s'en sert pour dire honnêtement
-- « la régulation ne tourne plus » au lieu d'afficher des valeurs figées.
-- L'IPX surveille le même signal de son côté et bascule en repli s'il
-- s'arrête : c'est ce qui permet de faire tourner le démon sur un
-- hyperviseur sans mettre la ventilation à la merci d'un redémarrage.
CREATE TABLE IF NOT EXISTS ambiance_heartbeat (
    id INTEGER PRIMARY KEY CHECK(id = 1),
    ts INTEGER NOT NULL,
    version TEXT,
    cycles INTEGER NOT NULL DEFAULT 0,
    erreurs INTEGER NOT NULL DEFAULT 0
);

-- Alertes d'ambiance. Une alerte reste ouverte (fin IS NULL) tant que la
-- cause dure : on ne crée pas une ligne par cycle, sinon la table devient
-- illisible le jour où elle sert vraiment.
CREATE TABLE IF NOT EXISTS ambiance_alerte (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    salle_id INTEGER REFERENCES salle(id),
    type TEXT NOT NULL,
    gravite TEXT NOT NULL DEFAULT 'alerte',
    debut TEXT NOT NULL,
    fin TEXT,
    message TEXT NOT NULL,
    acquittee INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_ambiance_alerte_ouverte
    ON ambiance_alerte(salle_id, type) WHERE fin IS NULL;
CREATE INDEX IF NOT EXISTS idx_ambiance_alerte_debut
    ON ambiance_alerte(debut);

-- Courbes de départ, reprises des consignes déjà en service. Elles ne sont
-- insérées que si la table est vide : une modification faite dans
-- l'application ne doit jamais être écrasée par un redémarrage.
INSERT INTO ambiance_courbe (nom, stade, note)
SELECT 'Maternité', 'maternite',
       'Consigne élevée à la mise-bas pour les porcelets, descendue progressivement pour la truie'
WHERE NOT EXISTS (SELECT 1 FROM ambiance_courbe);

INSERT INTO ambiance_courbe_point (courbe_id, jour, t_cible, ventil_min, ventil_max)
SELECT c.id, p.jour, p.t_cible, p.ventil_min, p.ventil_max
FROM ambiance_courbe c
JOIN (
    SELECT 0 AS jour, 24.0 AS t_cible, 40.0 AS ventil_min, 100.0 AS ventil_max
    UNION ALL SELECT 4, 23.0, 40.0, 100.0
    UNION ALL SELECT 10, 22.0, 40.0, 100.0
    UNION ALL SELECT 21, 21.0, 40.0, 100.0
    UNION ALL SELECT 28, 20.0, 40.0, 100.0
) p
WHERE c.stade = 'maternite'
  AND NOT EXISTS (SELECT 1 FROM ambiance_courbe_point WHERE courbe_id = c.id);

INSERT INTO ambiance_courbe (nom, stade, note)
SELECT 'Post-sevrage', 'post-sevrage',
       'Départ à 28 °C au sevrage, descente régulière jusqu''au transfert en engraissement'
WHERE NOT EXISTS (SELECT 1 FROM ambiance_courbe WHERE stade = 'post-sevrage');

INSERT INTO ambiance_courbe_point (courbe_id, jour, t_cible, ventil_min, ventil_max)
SELECT c.id, p.jour, p.t_cible, p.ventil_min, p.ventil_max
FROM ambiance_courbe c
JOIN (
    SELECT 0 AS jour, 28.0 AS t_cible, 25.0 AS ventil_min, 100.0 AS ventil_max
    UNION ALL SELECT 7, 27.0, 25.0, 100.0
    UNION ALL SELECT 14, 25.0, 25.0, 100.0
    UNION ALL SELECT 28, 23.0, 25.0, 100.0
    UNION ALL SELECT 42, 22.0, 25.0, 100.0
    UNION ALL SELECT 56, 21.0, 25.0, 100.0
) p
WHERE c.stade = 'post-sevrage'
  AND NOT EXISTS (SELECT 1 FROM ambiance_courbe_point WHERE courbe_id = c.id);

INSERT INTO ambiance_courbe (nom, stade, note)
SELECT 'Engraissement', 'engraissement',
       'Consigne descendante avec la croissance, les animaux produisent de plus en plus de chaleur'
WHERE NOT EXISTS (SELECT 1 FROM ambiance_courbe WHERE stade = 'engraissement');

INSERT INTO ambiance_courbe_point (courbe_id, jour, t_cible, ventil_min, ventil_max)
SELECT c.id, p.jour, p.t_cible, p.ventil_min, p.ventil_max
FROM ambiance_courbe c
JOIN (
    SELECT 0 AS jour, 22.0 AS t_cible, 20.0 AS ventil_min, 100.0 AS ventil_max
    UNION ALL SELECT 21, 21.0, 20.0, 100.0
    UNION ALL SELECT 49, 20.0, 20.0, 100.0
    UNION ALL SELECT 77, 19.0, 20.0, 100.0
) p
WHERE c.stade = 'engraissement'
  AND NOT EXISTS (SELECT 1 FROM ambiance_courbe_point WHERE courbe_id = c.id);

-- Carte de l'automate : les objets de l'IPX800 rangés par bloc matériel.
--
-- L'API renvoie plusieurs milliers d'objets à plat. Un éleveur ne cherche pas
-- « l'objet 262145 », il cherche « ma deuxième entrée analogique » ou « le
-- relais qui commande l'éclairage de la verraterie ». Ce découpage rend la
-- liste lisible.
--
-- Les bornes ne sont pas devinées : elles ont été relevées sur l'automate
-- (GET /api/core/io, objets préfixés [IPX]). Elles sont identiques sur tous
-- les IPX800 V5, la carte étant la même.
--
-- Attention à deux pièges que la documentation GCE ne signale pas :
--   * les relais ont DEUX plages, la commande (écriture) et l'état (lecture) ;
--     écrire sur l'état ne fait rien ;
--   * les sorties collecteur ouvert sont dans l'ordre inverse, l'état vient
--     AVANT la commande.
-- La documentation annonce par ailleurs des tableaux de 8 pour les opto et les
-- collecteurs ouverts ; l'automate n'en expose que 4 de chaque.

CREATE TABLE IF NOT EXISTS ipx_section (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nom TEXT NOT NULL,
    -- Source de lecture : 'ana' pour les valeurs analogiques, 'io' pour les
    -- états logiques.
    source TEXT NOT NULL DEFAULT 'io',
    -- Plage d'identifiants couverte. Laissée vide tant qu'elle est inconnue :
    -- la section s'affiche alors comme « à renseigner ».
    id_debut INTEGER,
    id_fin INTEGER,
    -- Les sorties se commandent, les entrées non. Détermine si le bouton de
    -- commande apparaît sur les lignes de cette section.
    commandable INTEGER NOT NULL DEFAULT 0,
    ordre INTEGER NOT NULL DEFAULT 0,
    note TEXT
);

CREATE INDEX IF NOT EXISTS idx_ipx_section_ordre ON ipx_section(ordre, nom);

INSERT INTO ipx_section (nom, source, id_debut, id_fin, commandable, ordre, note)
SELECT * FROM (
    SELECT 'Entrées analogiques' AS nom, 'ana' AS source, 262144 AS id_debut,
           262147 AS id_fin, 0 AS commandable, 10 AS ordre,
           'Les 4 entrées analogiques de la carte' AS note
    UNION ALL SELECT 'Sondes et extensions analogiques', 'ana', 262148, 999999, 0, 20,
           'X-THL et autres mesures des extensions'
    UNION ALL SELECT 'Sorties relais', 'io', 65536, 65543, 1, 30,
           'Les 8 relais de la carte — c''est ici que l''on commande'
    UNION ALL SELECT 'Retour d''état des relais', 'io', 65544, 65551, 0, 35,
           'Lecture seule : ce que le relais fait réellement'
    UNION ALL SELECT 'Entrées digitales', 'io', 65552, 65559, 0, 40,
           'Les 8 entrées digitales de la carte'
    UNION ALL SELECT 'Entrées opto-isolées', 'io', 65560, 65563, 0, 50,
           'Les 4 entrées opto-isolées de la carte'
    UNION ALL SELECT 'Sorties collecteur ouvert', 'io', 65568, 65571, 1, 60,
           'Les 4 sorties collecteur ouvert — c''est ici que l''on commande'
    UNION ALL SELECT 'Retour d''état des collecteurs ouverts', 'io', 65564, 65567, 0, 65,
           'Lecture seule'
    UNION ALL SELECT 'Commandes système', 'io', 65572, 65573, 1, 70,
           'Redémarrage de l''automate et bouton de sélection — à manipuler avec précaution'
)
WHERE NOT EXISTS (SELECT 1 FROM ipx_section);

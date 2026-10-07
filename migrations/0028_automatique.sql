-- Module Automatique : règles « si… alors… » qui relient les automates et
-- appareils de l'élevage (points IPX et Zigbee, salles, machine à soupe,
-- heure). Appliquées par eo-sevel (routes::automatique::boucle).
CREATE TABLE IF NOT EXISTS automatisme (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nom TEXT NOT NULL,
    -- 0 = arrêtée ; 1 = active.
    actif INTEGER NOT NULL DEFAULT 1,
    -- 1 = essai : la règle est évaluée et journalisée, mais n'agit sur rien.
    essai INTEGER NOT NULL DEFAULT 1,
    -- 'et' : toutes les conditions ; 'ou' : une seule suffit.
    logique TEXT NOT NULL DEFAULT 'et',
    -- La condition doit tenir ce nombre de secondes avant de déclencher.
    delai_s INTEGER NOT NULL DEFAULT 0,
    -- Plage horaire où la règle peut se déclencher (HH:MM), vide = toujours.
    heure_debut TEXT,
    heure_fin TEXT,
    note TEXT,
    cree_le INTEGER NOT NULL DEFAULT (strftime('%s','now'))
);

CREATE TABLE IF NOT EXISTS automatisme_condition (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    automatisme_id INTEGER NOT NULL REFERENCES automatisme(id) ON DELETE CASCADE,
    -- point | salle_t | salle_h | soupe_phase
    source TEXT NOT NULL,
    point_id INTEGER REFERENCES ipx_point(id) ON DELETE CASCADE,
    salle_id INTEGER REFERENCES salle(id) ON DELETE CASCADE,
    -- > < >= <= = !=
    operateur TEXT NOT NULL,
    valeur REAL NOT NULL
);

CREATE TABLE IF NOT EXISTS automatisme_action (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    automatisme_id INTEGER NOT NULL REFERENCES automatisme(id) ON DELETE CASCADE,
    -- debut : quand la règle se déclenche ; fin : quand la condition cesse.
    moment TEXT NOT NULL DEFAULT 'debut',
    -- commande | sms
    type TEXT NOT NULL,
    point_id INTEGER REFERENCES ipx_point(id) ON DELETE CASCADE,
    valeur REAL,
    message TEXT,
    ordre INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS automatisme_etat (
    automatisme_id INTEGER PRIMARY KEY REFERENCES automatisme(id) ON DELETE CASCADE,
    -- Instant où la condition est devenue vraie (NULL = fausse).
    vrai_depuis INTEGER,
    -- 1 tant que la règle est déclenchée (actions « début » faites).
    declenche INTEGER NOT NULL DEFAULT 0,
    dernier_declenchement INTEGER,
    -- Dernier problème de lecture (capteur muet…), pour l'afficher.
    probleme TEXT,
    maj INTEGER
);

CREATE TABLE IF NOT EXISTS automatisme_journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL DEFAULT (strftime('%s','now')),
    automatisme_id INTEGER,
    nom TEXT NOT NULL,
    evenement TEXT NOT NULL,
    detail TEXT,
    essai INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_automatisme_journal_ts ON automatisme_journal(ts);

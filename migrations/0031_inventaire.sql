-- Inventaire terrain (voir routes::inventaire) : une feuille par comptage,
-- une ligne par animal ou par case. `attendu` est figé à la création de la
-- feuille (ce que le logiciel compte à la date de l'inventaire), `trouve` est
-- reporté au retour du terrain (NULL = pas encore compté).
CREATE TABLE IF NOT EXISTS inventaire (
    id INTEGER PRIMARY KEY,
    date TEXT NOT NULL,
    note TEXT,
    cree_par TEXT,
    cree_le TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE IF NOT EXISTS inventaire_ligne (
    id INTEGER PRIMARY KEY,
    inventaire_id INTEGER NOT NULL REFERENCES inventaire(id) ON DELETE CASCADE,
    ordre INTEGER NOT NULL,
    section TEXT NOT NULL,
    site TEXT,
    salle TEXT,
    emplacement TEXT,
    designation TEXT NOT NULL,
    detail TEXT,
    attendu INTEGER NOT NULL,
    trouve INTEGER,
    remarque TEXT
);
CREATE INDEX IF NOT EXISTS inventaire_ligne_inventaire ON inventaire_ligne(inventaire_id, ordre);

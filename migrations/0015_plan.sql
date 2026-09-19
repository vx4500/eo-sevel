-- Plan du site : image de fond (une par installation, comme le logo de
-- vente directe) et blocs libres qu'un administrateur peut positionner sur
-- le plan à côté des salles (info, raccourci...).
CREATE TABLE IF NOT EXISTS plan_reglage (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    image_data BLOB,
    image_mime TEXT
);

CREATE TABLE IF NOT EXISTS plan_bloc (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    titre TEXT NOT NULL DEFAULT '',
    contenu TEXT NOT NULL DEFAULT '',
    lien_url TEXT,
    x REAL NOT NULL,
    y REAL NOT NULL,
    w REAL NOT NULL,
    h REAL NOT NULL,
    rotation REAL NOT NULL DEFAULT 0
);

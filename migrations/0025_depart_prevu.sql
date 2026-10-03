-- Départs de porcs prévus (camion d'enlèvement) : date et heure, nombre
-- annoncé. Les salles d'où partent les porcs ne sont demandées qu'au moment
-- de confirmer le départ, une fois l'heure passée (voir src/routes/departs.rs).
CREATE TABLE IF NOT EXISTS depart_prevu (
    id INTEGER PRIMARY KEY,
    date_heure TEXT NOT NULL,          -- AAAA-MM-JJTHH:MM, heure locale
    nombre INTEGER NOT NULL,
    note TEXT,
    fait INTEGER NOT NULL DEFAULT 0,   -- 1 = confirmé (sorties enregistrées)
    nombre_reel INTEGER,
    cree_le TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

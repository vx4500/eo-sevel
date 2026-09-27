-- Démarrages d'une machine vus par un détecteur de vibration Zigbee
-- (compresseur…) : chaque passage « à l'arrêt » → « en marche » est compté.
-- Alerte quand le nombre de démarrages sur la période dépasse le maximum.
CREATE TABLE IF NOT EXISTS zigbee_demarrage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ieee TEXT NOT NULL,
    ts INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_zigbee_demarrage ON zigbee_demarrage(ieee, ts);

CREATE TABLE IF NOT EXISTS zigbee_demarrage_reglage (
    ieee TEXT PRIMARY KEY,
    -- Alerte au-delà de ce nombre de démarrages ; vide = pas d'alerte.
    demarrages_max INTEGER,
    periode_min INTEGER NOT NULL DEFAULT 60
);

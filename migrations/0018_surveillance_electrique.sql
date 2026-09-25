-- Surveillance électrique des appareils mesurés par un compteur Zigbee
-- (machine à soupe, compteur général…). Réglages par appareil ; l'état de
-- marche et les périodes de marche sont tenus par le démon `eo-regul`.

CREATE TABLE IF NOT EXISTS zigbee_surveillance (
    ieee TEXT PRIMARY KEY,
    -- Au-dessus de cette puissance, l'appareil est considéré en marche.
    seuil_marche_w REAL NOT NULL DEFAULT 20,
    -- Alertes, désactivées quand vides.
    puissance_max_w REAL,
    marche_max_min INTEGER,
    arret_max_h REAL,
    -- Coupure de phase : une tension sous ce seuil sur une phase.
    phases INTEGER NOT NULL DEFAULT 1,
    tension_min_v REAL NOT NULL DEFAULT 190,
    -- État tenu par le démon.
    en_marche INTEGER,
    depuis INTEGER
);

CREATE TABLE IF NOT EXISTS zigbee_marche (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ieee TEXT NOT NULL,
    debut INTEGER NOT NULL,
    fin INTEGER,
    energie_debut REAL,
    energie_fin REAL
);
CREATE INDEX IF NOT EXISTS idx_zigbee_marche ON zigbee_marche(ieee, debut);

-- Consommation par heure importée (historique Home Assistant d'avant le
-- passage des compteurs sur EO_Sevel) : sert à la vue Jour quand il n'y a
-- pas de mesures de puissance.
CREATE TABLE IF NOT EXISTS elec_heure (
    ieee TEXT NOT NULL,
    ts INTEGER NOT NULL,
    kwh REAL NOT NULL,
    -- 1 si la valeur a été répartie après un silence du compteur.
    estime INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (ieee, ts)
) WITHOUT ROWID;

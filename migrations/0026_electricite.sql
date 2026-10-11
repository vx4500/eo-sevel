-- Consommation électrique par jour et par compteur Zigbee, calculée en
-- intégrant la puissance mesurée. Gardée sans limite de durée : l'historique
-- détaillé (zigbee_mesure) est effacé au bout de 90 jours.
CREATE TABLE IF NOT EXISTS elec_jour (
    ieee TEXT NOT NULL,
    jour TEXT NOT NULL,
    kwh REAL NOT NULL,
    -- Part de la journée couverte par des mesures (0 à 1).
    couverture REAL NOT NULL,
    PRIMARY KEY (ieee, jour)
) WITHOUT ROWID;

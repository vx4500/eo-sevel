-- Plan de l'élevage en un coup d'œil (/ambiance/plan) : repères posés sur
-- l'image du plan (plan_reglage, 0015). Les coordonnées sont en pour cent de
-- l'image, pour rester justes quelle que soit la taille de l'écran.
--
-- type : 'salle' (carte complète), 'temperature', 'humidite', 'ventil',
-- 'mode' (mode de ventilation), 'eclairage' (programme d'éclairage), 'texte'.
-- ref_id : salle.id, ou eclairage_programme.id pour 'eclairage', NULL pour 'texte'.
CREATE TABLE IF NOT EXISTS plan_repere (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    type TEXT NOT NULL,
    ref_id INTEGER,
    libelle TEXT,
    x REAL NOT NULL DEFAULT 50,
    y REAL NOT NULL DEFAULT 50
);

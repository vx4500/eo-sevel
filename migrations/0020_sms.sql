-- Historique des SMS envoyés par l'option Free Mobile (voir src/sms_free.rs).
CREATE TABLE IF NOT EXISTS sms_envoi (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    horodatage TEXT NOT NULL,
    origine TEXT NOT NULL,
    message TEXT NOT NULL,
    statut TEXT NOT NULL
);

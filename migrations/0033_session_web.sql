-- Sessions de connexion conservées en base : une mise à jour ou un
-- redémarrage du service ne déconnecte plus personne (voir routes::session_web).
-- vu_le = dernière requête (horodatage Unix) ; une session inactive plus de
-- auth::INACTIVITE_MAX_S est fermée, en mémoire comme ici.
CREATE TABLE IF NOT EXISTS session_web (
    id TEXT PRIMARY KEY,
    uid INTEGER NOT NULL,
    cree_le INTEGER NOT NULL,
    vu_le INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS session_web_uid ON session_web(uid);

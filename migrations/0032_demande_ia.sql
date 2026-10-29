-- Demandes de modification du logiciel envoyées à l'IA depuis une note du
-- plan (voir routes::demandes_ia et /usr/local/sbin/eo-demande-ia).
-- statut : attente → en_cours → prete → installation → installee
--          (ou echec, annulee). La branche git `demande-<id>` porte la
-- modification tant qu'elle n'est pas installée.
CREATE TABLE IF NOT EXISTS demande_ia (
    id INTEGER PRIMARY KEY,
    repere_id INTEGER,
    texte TEXT NOT NULL,
    auteur TEXT,
    cree_le TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    statut TEXT NOT NULL DEFAULT 'attente',
    compte_rendu TEXT,
    version TEXT,
    maj_le TEXT
);
CREATE INDEX IF NOT EXISTS demande_ia_statut ON demande_ia(statut, id);

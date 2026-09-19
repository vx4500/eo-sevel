-- Observations comportementales (EO_Sevel).
--
-- Saisie rapide volontairement pauvre en champs : ce qui ne se déduit pas
-- (cannibalisme, queue/oreille mordue, gravité) est demandé ; la salle se
-- déduit de la case quand elle est connue, pour pouvoir croiser ensuite
-- l'observation avec le climat du jour dans `ambiance_jour` sans ressaisie.
CREATE TABLE IF NOT EXISTS observation_comportement (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    bande_id INTEGER NOT NULL REFERENCES bande(id),
    case_id INTEGER REFERENCES casesalle(id),
    salle_id INTEGER REFERENCES salle(id),

    -- 'cannibalisme', 'queue_mordue', 'oreille_mordue' ou 'autre' — texte et
    -- non enum SQLite pour rester additif si une catégorie s'ajoute plus tard.
    categorie TEXT NOT NULL,

    -- 'debut', 'aggravation' ou 'perte_inexpliquee' : la même observation
    -- suivie dans le temps peut changer de gravité sans qu'on efface
    -- l'historique, chaque saisie reste une ligne distincte.
    gravite TEXT NOT NULL DEFAULT 'debut',

    nombre INTEGER,
    note TEXT,
    utilisateur TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_observation_comportement_salle_date
    ON observation_comportement(salle_id, date);
CREATE INDEX IF NOT EXISTS idx_observation_comportement_bande
    ON observation_comportement(bande_id, date);

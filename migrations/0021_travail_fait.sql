-- Travail de la semaine : truies pointées « fait » étape par étape
-- (déplacements, soins, retours…). Les étapes qui ont un vrai événement
-- (IA, écho, mise-bas, sevrage) se valident par cet événement.
CREATE TABLE IF NOT EXISTS travail_fait (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    code TEXT NOT NULL,
    acte_id INTEGER NOT NULL DEFAULT 0,
    bande_id INTEGER NOT NULL REFERENCES bande(id),
    truie_id INTEGER NOT NULL REFERENCES truie(id),
    date_fait TEXT NOT NULL,
    par TEXT,
    horodatage TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(code, acte_id, bande_id, truie_id)
);

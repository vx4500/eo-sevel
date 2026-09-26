-- Prestataires (engraisseurs à façon) : bandes confiées, options par
-- prestataire, aliment consommé et pesées saisis depuis leur portail.
-- L'inventaire réutilise mouvementstock (type_saisie='inventaire', est_stock=1)
-- et les sorties mortes declarationmort (+ num_equarrissage, colonne ajoutée
-- par ensure_column).

CREATE TABLE IF NOT EXISTS prestataire_option (
    utilisateur_id INTEGER PRIMARY KEY REFERENCES utilisateur(id),
    -- 1 : le n° de bon d'équarrissage est exigé à chaque sortie morte.
    equarrissage_obligatoire INTEGER NOT NULL DEFAULT 0
);

-- Historique des bandes confiées ; bande.engraisseur_id reste le
-- prestataire en cours (un seul à la fois).
CREATE TABLE IF NOT EXISTS bande_prestataire (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    bande_id INTEGER NOT NULL REFERENCES bande(id),
    utilisateur_id INTEGER NOT NULL REFERENCES utilisateur(id),
    date_debut TEXT NOT NULL,
    date_fin TEXT
);
CREATE INDEX IF NOT EXISTS ix_bande_prestataire ON bande_prestataire(bande_id, date_debut);

-- Aliment : « livraison » (livraison ou vidage de silo, un jour) ou
-- « releve » (quantité consommée sur une période date_debut → date).
CREATE TABLE IF NOT EXISTS consoaliment_bande (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    horodatage TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    bande_code TEXT NOT NULL,
    type TEXT NOT NULL DEFAULT 'livraison',
    date_debut TEXT,
    date TEXT NOT NULL,
    aliment TEXT,
    quantite_kg REAL NOT NULL,
    declare_par TEXT,
    note TEXT
);
CREATE INDEX IF NOT EXISTS ix_consoaliment_bande ON consoaliment_bande(bande_code, date);

CREATE TABLE IF NOT EXISTS pesee_bande (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    horodatage TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    bande_code TEXT NOT NULL,
    date TEXT NOT NULL,
    nombre INTEGER NOT NULL,
    poids_moyen REAL NOT NULL,
    declare_par TEXT,
    note TEXT
);
CREATE INDEX IF NOT EXISTS ix_pesee_bande ON pesee_bande(bande_code, date);

-- Début des pesées (jours après la mise-bas), à côté des autres étapes.
INSERT OR IGNORE INTO reglage(cle,valeur,libelle) VALUES
('debut_pesees',180,'Début des pesées avant départ (j)');

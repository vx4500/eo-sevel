-- Machine à soupe Asserva (Exa-Feeder) suivie en écoute seule : un petit
-- service sur l'hôte Proxmox lit la liaison série du PC Mega-feeder et
-- transmet les trames (voir src/soupe.rs). EO_Sevel n'envoie jamais rien.

-- Dernier état connu (une seule ligne).
CREATE TABLE IF NOT EXISTS soupe_etat (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    ts INTEGER NOT NULL,
    phase INTEGER NOT NULL,
    poids INTEGER,
    sequence INTEGER,
    formule INTEGER,
    temps_min INTEGER,
    vanne INTEGER,
    consigne REAL,
    controle REAL,
    debit REAL,
    rpm INTEGER,
    passage INTEGER,
    produit INTEGER,
    doit INTEGER,
    seuil INTEGER,
    cumul INTEGER
);

-- Quantité servie à chaque vanne, passage par passage.
CREATE TABLE IF NOT EXISTS soupe_distribution (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    jour TEXT NOT NULL,
    sequence INTEGER,
    formule INTEGER,
    passage INTEGER,
    vanne INTEGER NOT NULL,
    consigne REAL,
    distribue REAL
);
CREATE INDEX IF NOT EXISTS idx_soupe_distribution_jour ON soupe_distribution(jour);

-- Changements de phase et actions faites depuis le PC.
CREATE TABLE IF NOT EXISTS soupe_evenement (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts INTEGER NOT NULL,
    phase INTEGER,
    message TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_soupe_evenement_ts ON soupe_evenement(ts);

-- Dernière réponse brute de la machine pour chaque commande de lecture
-- (fiches vannes R, plans S, horaires U, système V, relais W, produits X,
-- formules Y…), relevée quand le PC synchronise ses données.
CREATE TABLE IF NOT EXISTS soupe_donnee (
    commande TEXT PRIMARY KEY,
    ts INTEGER NOT NULL,
    contenu TEXT NOT NULL
);

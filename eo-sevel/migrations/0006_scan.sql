-- Import par scan d'emballage (EO_Sevel).
--
-- Un code scanné donne un numéro de produit, un lot et une péremption. Il ne
-- donne jamais le nom du produit : un GTIN est un numéro, et il n'existe pas
-- de base publique fiable côté vétérinaire. La correspondance s'apprend donc
-- au premier scan, et ne se redemande plus jamais ensuite.

CREATE TABLE IF NOT EXISTS code_produit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Clé de reconnaissance : le GTIN quand le code en contient un, sinon la
    -- chaîne brute. Un code-barres non normalisé reste ainsi utilisable.
    cle TEXT NOT NULL UNIQUE,
    -- Destination : pour l'instant la pharmacie, l'aliment viendra ensuite.
    domaine TEXT NOT NULL DEFAULT 'pharmacie',
    -- Nom du produit tel qu'il existe déjà dans produitpharmacie. On stocke le
    -- libellé plutôt qu'un identifiant, parce que c'est ainsi que les
    -- mouvements de pharmacie référencent les produits dans cette base.
    produit TEXT NOT NULL,
    -- Conditionnement : combien d'unités de stock pour une boîte scannée.
    -- Une boîte de 50 doses vaut 50 doses ; scanner une boîte doit ajouter 50,
    -- pas 1.
    unites_par_boite REAL NOT NULL DEFAULT 1,
    libelle_fabricant TEXT,
    note TEXT,
    cree_le TEXT NOT NULL DEFAULT (datetime('now')),
    dernier_scan TEXT,
    nb_scans INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_code_produit_domaine ON code_produit(domaine, produit);

-- Journal des scans, y compris ceux qui n'ont abouti à rien. Sans lui, un code
-- mal lu ou refusé ne laisse aucune trace et le problème est invisible.
CREATE TABLE IF NOT EXISTS scan_journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    horodatage TEXT NOT NULL DEFAULT (datetime('now')),
    utilisateur TEXT,
    domaine TEXT NOT NULL DEFAULT 'pharmacie',
    code_brut TEXT NOT NULL,
    gtin TEXT,
    lot TEXT,
    peremption TEXT,
    reconnu INTEGER NOT NULL DEFAULT 0,
    resultat TEXT
);

CREATE INDEX IF NOT EXISTS idx_scan_journal_date ON scan_journal(horodatage);

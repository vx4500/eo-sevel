-- Suivi des cours (page Quotidien, voir src/cours.rs) : un relevé par
-- produit et par date de marché ; `prix` en €/kg pour le porc, en €/t pour
-- les céréales.
CREATE TABLE IF NOT EXISTS cours_releve (
    produit TEXT NOT NULL,
    date TEXT NOT NULL,
    prix REAL NOT NULL,
    releve_le TEXT NOT NULL,
    PRIMARY KEY (produit, date)
);

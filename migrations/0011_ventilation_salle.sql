-- Ventilateurs déclarés par salle (EO_Sevel).
--
-- Objectif : dimensionner la ventilation indépendamment de la régulation
-- (0005_ambiance.sql). Une salle peut avoir plusieurs ventilateurs — par
-- exemple une extraction plafond et une extraction fosse — chacun avec sa
-- propre fonction et son propre débit. Le taux de renouvellement d'air
-- (débit total ÷ m³ de la salle) se déduit ensuite en croisant cette table
-- avec les colonnes longueur/largeur/hauteur de `salle`.
CREATE TABLE IF NOT EXISTS ventilateur_salle (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    salle_id INTEGER NOT NULL REFERENCES salle(id),
    nom TEXT NOT NULL,

    -- 'interieur' (brasseur, gaine de reprise...) ou 'exterieur' (tourelle,
    -- caisson en façade...).
    position TEXT NOT NULL DEFAULT 'exterieur',

    -- 'extraction' ou 'insufflation' : deux logiques de régulation
    -- différentes (dépression vs surpression).
    fonction TEXT NOT NULL DEFAULT 'extraction',

    -- Voie de passage de l'air : 'plafond', 'toiture', 'fosse', 'pignon',
    -- 'gaine'... texte libre pour ne pas figer un vocabulaire qui varie
    -- selon les bâtiments.
    voie TEXT,

    diametre_mm REAL,
    -- Débit nominal du constructeur si connu ; sinon estimé à partir du
    -- diamètre côté application. Les deux peuvent coexister (le débit réel
    -- prime sur l'estimation géométrique).
    debit_m3h REAL,

    -- Adressage IPX800 V5 de la sortie qui pilote ce ventilateur, texte comme
    -- dans ambiance_salle pour rester indépendant du numérotage firmware.
    ipx_sortie TEXT,

    actif INTEGER NOT NULL DEFAULT 1,
    note TEXT
);

CREATE INDEX IF NOT EXISTS idx_ventilateur_salle_salle ON ventilateur_salle(salle_id);

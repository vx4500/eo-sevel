-- Inventaire des points de l'automate IPX800 (EO_Sevel).
--
-- Un « point » est une entrée ou une sortie de l'automate à laquelle on a donné
-- un nom et un rôle. Sans cet inventaire, l'automate n'expose que des chemins
-- d'API : lisibles par une machine, illisibles pour un éleveur six mois plus
-- tard.
--
-- Les points de type compteur ne créent pas une nouvelle mécanique de suivi :
-- ils alimentent les compteurs d'eau et d'électricité qui existent déjà dans le
-- logiciel, avec leur répartition de coût par bande. Un relevé saisi à la main
-- et un relevé lu sur l'automate sont la même chose.

CREATE TABLE IF NOT EXISTS ipx_point (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hote TEXT NOT NULL,
    -- Chemin d'API de lecture, relevé dans l'explorateur.
    chemin TEXT NOT NULL,
    -- Chemin d'écriture, quand le point se commande. Souvent identique au
    -- chemin de lecture, mais pas toujours : on ne le devine pas.
    chemin_ecriture TEXT,
    nom TEXT NOT NULL,
    -- temperature | humidite | luminosite | ventilation | eclairage |
    -- disjoncteur | compteur_eau | compteur_elec | autre
    role TEXT NOT NULL DEFAULT 'autre',
    salle_id INTEGER REFERENCES salle(id),
    unite TEXT,
    -- Conversion de la valeur brute vers l'unité réelle :
    -- valeur = brut * coefficient + decalage.
    -- C'est ici qu'on écrit « une impulsion vaut dix litres ».
    coefficient REAL NOT NULL DEFAULT 1,
    decalage REAL NOT NULL DEFAULT 0,
    -- Compteur d'eau ou d'électricité alimenté par ce point.
    compteur_id INTEGER REFERENCES compteur_energie(id),
    -- Un relevé par jour suffit pour un compteur ; en dessous, on accumule des
    -- lignes sans rien apprendre de plus.
    releve_heure INTEGER NOT NULL DEFAULT 6,
    dernier_releve TEXT,
    pilotable INTEGER NOT NULL DEFAULT 0,
    actif INTEGER NOT NULL DEFAULT 1,
    ordre INTEGER NOT NULL DEFAULT 0,
    note TEXT,
    cree_le TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(hote, chemin)
);

CREATE INDEX IF NOT EXISTS idx_ipx_point_role ON ipx_point(role, ordre, nom);
CREATE INDEX IF NOT EXISTS idx_ipx_point_compteur ON ipx_point(compteur_id);

-- Toute commande manuelle envoyée à l'automate depuis l'interface. Piloter une
-- sortie depuis une page web engage du matériel réel : il faut pouvoir dire
-- ensuite qui a fait quoi, et quand.
CREATE TABLE IF NOT EXISTS ipx_commande_journal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    horodatage TEXT NOT NULL DEFAULT (datetime('now')),
    utilisateur TEXT,
    point_id INTEGER REFERENCES ipx_point(id),
    nom_point TEXT,
    valeur REAL,
    resultat TEXT
);

CREATE INDEX IF NOT EXISTS idx_ipx_commande_date ON ipx_commande_journal(horodatage);

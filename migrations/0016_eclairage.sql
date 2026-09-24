-- Programmes d'éclairage (EO_Sevel).
--
-- Un programme allume et éteint une ou plusieurs sorties de l'automate selon
-- des plages horaires, la luminosité mesurée et une minuterie. La page
-- (/ambiance/eclairage) écrit la configuration ; le démon `eo-regul` la
-- relit à chaque cycle, décide avec `eclairage_core` et écrit sur l'automate.
-- Même séparation que pour la ventilation : l'éclairage continue de suivre
-- son programme quand l'application est arrêtée ou mise à jour.

CREATE TABLE IF NOT EXISTS eclairage_programme (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    nom TEXT NOT NULL,
    actif INTEGER NOT NULL DEFAULT 1,
    -- Mode essai : le démon calcule et journalise ce qu'il aurait fait, sans
    -- rien envoyer à l'automate. Coché par défaut sur un nouveau programme.
    essai INTEGER NOT NULL DEFAULT 1,
    -- Luminosité : capteur (point de rôle « luminosite ») et seuils. Sans
    -- capteur, la luminosité n'intervient pas.
    lux_point_id INTEGER REFERENCES ipx_point(id) ON DELETE SET NULL,
    lux_allumer_sous REAL,
    lux_eteindre_au_dessus REAL,
    lux_temporisation_min INTEGER NOT NULL DEFAULT 5,
    -- horaire | eteindre | garder : voir SiCapteurMuet.
    lux_si_muet TEXT NOT NULL DEFAULT 'horaire',
    -- Extinction automatique après ce nombre de minutes d'allumage.
    minuterie_min INTEGER,
    minuterie_manuel INTEGER NOT NULL DEFAULT 1,
    -- Valeurs envoyées à la sortie. 1/0 pour un relais ; un variateur peut
    -- recevoir 60 pour 60 %.
    valeur_allume REAL NOT NULL DEFAULT 1,
    valeur_eteint REAL NOT NULL DEFAULT 0,
    -- Forçage depuis la page : 1 allumé, 0 éteint, jusqu'à forcage_fin
    -- (horodatage Unix). NULL : pas de forçage.
    forcage INTEGER,
    forcage_fin INTEGER,
    note TEXT,
    cree_le TEXT NOT NULL DEFAULT (datetime('now')),
    modifie_le TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Plages horaires. jours : sept caractères 0/1, lundi en premier. debut et
-- fin en minutes depuis minuit ; fin < debut passe minuit.
CREATE TABLE IF NOT EXISTS eclairage_plage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    programme_id INTEGER NOT NULL REFERENCES eclairage_programme(id) ON DELETE CASCADE,
    jours TEXT NOT NULL DEFAULT '1111111',
    debut INTEGER NOT NULL,
    fin INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_eclairage_plage_programme ON eclairage_plage(programme_id);

-- Sorties commandées. Une sortie n'appartient qu'à un seul programme : deux
-- programmes sur la même lampe se contrediraient.
CREATE TABLE IF NOT EXISTS eclairage_sortie (
    programme_id INTEGER NOT NULL REFERENCES eclairage_programme(id) ON DELETE CASCADE,
    point_id INTEGER NOT NULL UNIQUE REFERENCES ipx_point(id) ON DELETE CASCADE,
    PRIMARY KEY (programme_id, point_id)
);

-- État tenu par le démon, une ligne par sortie. La page l'affiche ; effacer
-- les lignes d'un programme (après une modification) fait réappliquer sa
-- demande au cycle suivant.
CREATE TABLE IF NOT EXISTS eclairage_etat (
    point_id INTEGER PRIMARY KEY REFERENCES ipx_point(id) ON DELETE CASCADE,
    programme_id INTEGER REFERENCES eclairage_programme(id) ON DELETE CASCADE,
    demande INTEGER,
    sombre INTEGER,
    bascule_depuis INTEGER,
    allume INTEGER,
    allume_depuis INTEGER,
    par_programme INTEGER NOT NULL DEFAULT 0,
    force INTEGER NOT NULL DEFAULT 0,
    lux REAL,
    raison TEXT,
    erreur TEXT,
    maj INTEGER
);

-- Réseau Zigbee (EO_Sevel), via Zigbee2MQTT et Mosquitto sur le serveur.
--
-- Le démon `eo-regul` écoute Zigbee2MQTT : il tient ici la liste des
-- appareils, leurs dernières valeurs et un court historique des mesures, et
-- transmet les commandes déposées dans `zigbee_commande`. L'application ne
-- parle jamais au MQTT : elle lit ces tables et écrit dans la file.
--
-- Un appareil rattaché à une salle devient un ou plusieurs points de
-- `ipx_point` avec hote='zigbee' et chemin='<ieee>/<grandeur>' : l'éclairage,
-- les compteurs, Mes salles et les alertes le traitent alors comme une entrée
-- de l'automate, sans mécanique en double.

CREATE TABLE IF NOT EXISTS zigbee_appareil (
    ieee TEXT PRIMARY KEY,
    nom TEXT NOT NULL,
    modele TEXT,
    fabricant TEXT,
    description TEXT,
    -- Router | EndDevice
    type_reseau TEXT,
    alimentation TEXT,
    -- sonde | interrupteur | variateur | compteur | vibration | autre
    categorie TEXT NOT NULL DEFAULT 'autre',
    -- Grandeurs exposées, séparées par des virgules (temperature,humidity…).
    grandeurs TEXT,
    disponible INTEGER,
    vu_le INTEGER,
    batterie REAL,
    signal INTEGER,
    -- Seuils d'alerte de température propres à l'appareil (frigo : 2 à 8 °C).
    t_min REAL,
    t_max REAL,
    present INTEGER NOT NULL DEFAULT 1,
    maj INTEGER
);

-- Dernière valeur de chaque grandeur. Les états texte (ON/OFF) sont aussi
-- traduits en nombre (1/0) pour les lectures numériques.
CREATE TABLE IF NOT EXISTS zigbee_valeur (
    ieee TEXT NOT NULL,
    cle TEXT NOT NULL,
    valeur REAL,
    texte TEXT,
    ts INTEGER NOT NULL,
    PRIMARY KEY (ieee, cle)
) WITHOUT ROWID;

-- Historique court des grandeurs lentes (température, humidité, batterie),
-- pour la frise de Mes salles. Purgé au-delà de 90 jours.
CREATE TABLE IF NOT EXISTS zigbee_mesure (
    ieee TEXT NOT NULL,
    cle TEXT NOT NULL,
    ts INTEGER NOT NULL,
    valeur REAL NOT NULL,
    PRIMARY KEY (ieee, cle, ts)
) WITHOUT ROWID;

-- Commandes à publier. `charge` est le JSON envoyé à
-- zigbee2mqtt/<ieee>/set ; `envoye` est renseigné par le démon.
CREATE TABLE IF NOT EXISTS zigbee_commande (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ieee TEXT NOT NULL,
    charge TEXT NOT NULL,
    demande INTEGER NOT NULL,
    envoye INTEGER,
    resultat TEXT
);
CREATE INDEX IF NOT EXISTS idx_zigbee_commande_attente ON zigbee_commande(envoye) WHERE envoye IS NULL;

-- Battement du pont MQTT, distinct de celui de la régulation.
CREATE TABLE IF NOT EXISTS zigbee_pont (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    connecte INTEGER NOT NULL DEFAULT 0,
    ts INTEGER NOT NULL,
    messages INTEGER NOT NULL DEFAULT 0,
    erreur TEXT
);

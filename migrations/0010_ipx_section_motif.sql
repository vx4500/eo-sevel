-- Ranger les objets de l'automate par NOM et plus seulement par plage.
--
-- Les plages ne suffisent pas : l'IPX attribue ses identifiants dans l'ordre de
-- création des objets, si bien que les sorties d'un variateur, les défauts
-- d'une sonde et les résultats d'une fonction de régulation se retrouvent
-- mélangés dans une même plage. Ce qui les distingue de façon fiable, c'est le
-- préfixe que l'automate met dans leur nom : [x-010v-1], [X-THL-MATER-1],
-- [WEATHER], ]Enable, ]Result…
--
-- `motif` contient une liste de fragments séparés par des points-virgules. Un
-- objet rejoint la section si son nom contient l'un d'eux, sans tenir compte de
-- la casse. Une section qui porte un motif ignore sa plage.
--
-- Les sections sont examinées dans l'ordre `ordre` croissant et un objet ne
-- rejoint que la PREMIÈRE qui l'accepte : il ne peut donc pas apparaître à deux
-- endroits, et un motif large placé en fin de liste ne vole pas les objets
-- d'une section précise placée avant lui.

-- La colonne `motif` elle-même est ajoutée par la boucle additive de db.rs,
-- qui ignore une colonne déjà présente.

-- Sections nommées, ajoutées seulement si elles n'existent pas déjà.
INSERT INTO ipx_section (nom, source, motif, id_debut, id_fin, commandable, ordre, note)
SELECT * FROM (
    SELECT 'Sondes X-THL' AS nom, 'ana' AS source, 'X-THL;XTHL' AS motif,
           NULL AS id_debut, NULL AS id_fin, 0 AS commandable, 15 AS ordre,
           'Températures, hygrométries et luminosités des sondes' AS note
    UNION ALL SELECT 'Variateurs 0-10V — consignes et niveaux', 'ana', 'x-010v', NULL, NULL, 0, 22,
           'Consigne envoyée et tension réellement appliquée, en pourcentage'
    UNION ALL SELECT 'Météo', 'ana', '[WEATHER]', NULL, NULL, 0, 24,
           'Prévisions du greffon météo'
    UNION ALL SELECT 'Diagnostic', 'ana', '[DIAG]', NULL, NULL, 0, 26,
           'Compteurs internes de l''automate'
    UNION ALL SELECT 'Alimentation secourue', 'io', '[XPSU]', NULL, NULL, 0, 75,
           'Présence du secteur sur l''alimentation XPSU'
    UNION ALL SELECT 'Variateurs 0-10V — marche/arrêt', 'io', 'x-010v', NULL, NULL, 1, 80,
           'Mise en route de chaque voie de variateur'
    UNION ALL SELECT 'Défauts des sondes', 'io', ']Fault', NULL, NULL, 0, 85,
           'Un défaut à « marche » signale une sonde qui ne répond plus'
    UNION ALL SELECT 'Météo (jour/nuit)', 'io', '[WEATHER]', NULL, NULL, 0, 90,
           'Bascule jour/nuit du greffon météo'
    UNION ALL SELECT 'Logique interne de l''automate', 'io',
           ']Enable;]Result;]Output;]Start;]Enable/Rst;]Run/Pause;]Reset;]On/Off', NULL, NULL, 0, 200,
           'Fonctions, comparateurs, chronos et plannings programmés dans l''IPX. Cette logique a vocation à être reprise par EO_Sevel.'
    UNION ALL SELECT 'Bits internes', 'io', '_BANDE;_FORCE_BTN;ECL_', NULL, NULL, 1, 210,
           'Entrées/sorties virtuelles créées à la main dans l''automate'
) AS nouvelles
WHERE NOT EXISTS (SELECT 1 FROM ipx_section WHERE ipx_section.nom = nouvelles.nom);

-- La section fourre-tout analogique ne doit plus attraper les sondes : elle
-- passe après elles et ne récupère que ce qui reste.
UPDATE ipx_section SET nom='Autres mesures analogiques', ordre=28,
       note='Ce qui n''entre dans aucune catégorie plus précise'
 WHERE nom='Sondes et extensions analogiques';

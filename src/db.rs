use crate::auth;
use sqlx::{Row, SqlitePool};

/// Tables du socle historique (`0001_schema.sql`), stable depuis les bases
/// 1.55–1.65 et qui ne change jamais. C'est la référence utilisée par la
/// restauration de sauvegarde : contrairement à la base *actuelle*, qui
/// gagne une table à chaque fonctionnalité, ce socle n'a aucune raison de
/// manquer dans une sauvegarde authentique, même ancienne — `db::init`
/// complète toujours le reste tout seul au redémarrage.
pub const TABLES_SOCLE: &[&str] = &[
    "utilisateur",
    "site",
    "salle",
    "casesalle",
    "numeromarquage",
    "verrat",
    "truie",
    "bande",
    "evenement",
    "acteprotocole",
    "acterealise",
    "acterealiseverrat",
    "soinportee",
    "reglage",
    "parametre",
    "planaliment",
    "causeperte",
    "perteporcelet",
    "porccharcutier",
    "transfert",
    "livraisonaliment",
    "achatveto",
    "venteapport",
    "achatsemence",
    "achatgenetique",
    "affectationfacturebande",
    "affectationfacturecontrole",
    "objectif",
    "mesuretruie",
    "traitementcharcutier",
    "referenceifip",
    "mouvementstock",
    "journal",
    "entretien",
    "declarationmort",
    "saisieabattoir",
    "produitventedirecte",
    "reglageventedirecte",
    "sessionventedirecte",
    "coutelevageventedirecte",
    "chargeventedirecte",
    "clientventedirecte",
    "commandeventedirecte",
    "reglagecommunicationventedirecte",
    "messageventedirecte",
    "lignecommandeventedirecte",
    "produitpharmacie",
    "mouvementpharmacie",
    "traitementstock",
    "controlequotidien",
    "tache",
    "consosoupe",
    "porteerang",
    "cahiercharges",
    "valorisationapport",
    "compteur_energie",
    "releve_compteur",
    "importjournal",
    "importligne",
    "inventairecase",
    "receptionachat",
    "silo_aliment",
    "releve_silo",
    "consommationsoupe",
    "lignee_genetique",
    "demoobjet",
    "adoptionporcelet",
    "sortienourrice",
    "archive_nettoyage",
    "documentjustificatif",
];

pub async fn init(pool: &SqlitePool) -> anyhow::Result<()> {
    etablir_schema(pool).await?;
    crate::cleanup::historical_bands(pool).await?;
    achever_init(pool).await
}

/// Uniquement les migrations : tables, colonnes additives, index, vues. Rien
/// qui touche aux données (pas de compte admin, pas de réparation
/// économique, pas de nettoyage historique) — c'est ce que `sauvegarde` lit
/// pour savoir quel schéma une base fraîche possède aujourd'hui, sans
/// reproduire toute la logique d'initialisation.
pub async fn etablir_schema(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(include_str!("../migrations/0001_schema.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!("../migrations/0004_saisie_vocale.sql"))
        .execute(pool)
        .await?;
    // Ambiance et régulation de ventilation. Entièrement additive : une base
    // existante se met à niveau au premier démarrage sans rien perdre, et
    // aucune salle n'est régulée tant qu'elle n'a pas été activée à la main.
    sqlx::raw_sql(include_str!("../migrations/0005_ambiance.sql"))
        .execute(pool)
        .await?;
    // Import par scan d'emballage : correspondances apprises et journal.
    sqlx::raw_sql(include_str!("../migrations/0006_scan.sql"))
        .execute(pool)
        .await?;
    // Inventaire nommé des entrées et sorties de l'automate.
    sqlx::raw_sql(include_str!("../migrations/0007_ipx_points.sql"))
        .execute(pool)
        .await?;
    // Carte de l'automate : les objets rangés par bloc matériel.
    sqlx::raw_sql(include_str!("../migrations/0008_ipx_carte.sql"))
        .execute(pool)
        .await?;
    // Plages d'identifiants relevées sur l'automate, pour les bases où 0008
    // avait laissé les sections vides.
    sqlx::raw_sql(include_str!("../migrations/0009_ipx_carte_plages.sql"))
        .execute(pool)
        .await?;
    verify_sqlite_pragmas(pool).await?;

    // Migrations additives pour les bases anciennes 1.55–1.65. Une colonne déjà
    // présente est volontairement ignorée afin de rendre le démarrage idempotent.
    for (table, column, definition) in [
        ("ipx_section", "motif", "TEXT"),
        ("casesalle", "surface_config", "TEXT"),
        ("livraisonaliment", "date_reference", "TEXT"),
        (
            "livraisonaliment",
            "sites_json",
            "TEXT NOT NULL DEFAULT '[]'",
        ),
        ("achatveto", "date_reference", "TEXT"),
        ("achatveto", "sites_json", "TEXT NOT NULL DEFAULT '[]'"),
        ("achatgenetique", "ht_manuel", "INTEGER NOT NULL DEFAULT 0"),
        (
            "achatgenetique",
            "toutes_bandes",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "livraisonaliment",
            "stade_aliment",
            "TEXT NOT NULL DEFAULT 'auto'",
        ),
        ("controlequotidien", "salle_nom", "TEXT"),
        ("controlequotidien", "element", "TEXT"),
        ("controlequotidien", "utilisateur", "TEXT"),
        ("controlequotidien", "horodatage", "TEXT"),
        ("utilisateur", "sections", "TEXT"),
        (
            "utilisateur",
            "doit_changer_mdp",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        (
            "utilisateur",
            "tentatives_echec",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("utilisateur", "bloque_jusqu", "TEXT"),
        ("truie", "num_national", "TEXT"),
        ("truie", "nb_tetines", "INTEGER CHECK(nb_tetines>=0)"),
        ("truie", "splayleg", "INTEGER CHECK(splayleg>=0)"),
        ("truie", "bande_code", "TEXT"),
        ("truie", "perf_tx_perte", "REAL"),
        ("truie", "salle_id", "INTEGER"),
        ("truie", "case_id", "INTEGER"),
        ("truie", "source_import_id", "TEXT"),
        (
            "truie",
            "lignee_id",
            "INTEGER REFERENCES lignee_genetique(id)",
        ),
        ("bande", "cs_mn_portee", "REAL"),
        ("bande", "cs_adoptes", "INTEGER"),
        ("affectationfacturebande", "part", "REAL"),
        ("bande", "cs_retires", "INTEGER"),
        ("bande", "site", "TEXT"),
        ("livraisonaliment", "site", "TEXT"),
        ("achatveto", "site", "TEXT"),
        ("entretien", "site", "TEXT"),
        ("evenement", "resultat", "TEXT"),
        ("evenement", "chetifs", "INTEGER"),
        ("evenement", "ecrases", "INTEGER"),
        ("evenement", "tues_truie", "INTEGER"),
        ("evenement", "adoptes", "INTEGER"),
        ("evenement", "retires", "INTEGER"),
        ("evenement", "delai_attente", "INTEGER"),
        ("evenement", "nb_doses", "INTEGER"),
        ("evenement", "heure_debut", "TEXT"),
        ("evenement", "heure_fin", "TEXT"),
        ("evenement", "suivi_actif", "INTEGER NOT NULL DEFAULT 0"),
        ("evenement", "delivrance_ok", "INTEGER"),
        (
            "perteporcelet",
            "evenement_id",
            "INTEGER REFERENCES evenement(id) ON DELETE CASCADE",
        ),
        ("transfert", "vente_apport_id", "INTEGER"),
        ("declarationmort", "case_id", "INTEGER"),
        // N° du bon d'équarrissage (obligatoire selon l'option du prestataire).
        ("declarationmort", "num_equarrissage", "TEXT"),
        ("produitventedirecte", "quantite_disponible", "REAL"),
        ("produitventedirecte", "image_data", "BLOB"),
        ("produitventedirecte", "image_mime", "TEXT"),
        ("commandeventedirecte", "client_id", "INTEGER"),
        (
            "commandeventedirecte",
            "suivi_email",
            "INTEGER NOT NULL DEFAULT 1",
        ),
        (
            "commandeventedirecte",
            "suivi_sms",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("commandeventedirecte", "session_vente_id", "INTEGER"),
        (
            "commandeventedirecte",
            "nom_client",
            "TEXT NOT NULL DEFAULT ''",
        ),
        (
            "commandeventedirecte",
            "telephone",
            "TEXT NOT NULL DEFAULT ''",
        ),
        ("commandeventedirecte", "cree_le", "TEXT"),
        ("commandeventedirecte", "email", "TEXT"),
        ("commandeventedirecte", "notes", "TEXT"),
        (
            "commandeventedirecte",
            "statut",
            "TEXT NOT NULL DEFAULT 'nouvelle'",
        ),
        ("commandeventedirecte", "total", "REAL NOT NULL DEFAULT 0"),
        ("commandeventedirecte", "token_modification", "TEXT"),
        ("commandeventedirecte", "code_modification", "TEXT"),
        ("commandeventedirecte", "recap_envoye_le", "TEXT"),
        (
            "sessionventedirecte",
            "nom",
            "TEXT NOT NULL DEFAULT 'Session de vente'",
        ),
        ("sessionventedirecte", "date_creation", "TEXT"),
        ("sessionventedirecte", "date_livraison", "TEXT"),
        (
            "sessionventedirecte",
            "nb_porcs",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("sessionventedirecte", "bande_reference", "TEXT"),
        (
            "sessionventedirecte",
            "active",
            "INTEGER NOT NULL DEFAULT 1",
        ),
        ("sessionventedirecte", "notes", "TEXT"),
        ("sessionventedirecte", "date_cloture", "TEXT"),
        ("sessionventedirecte", "date_limite_commandes", "TEXT"),
        ("coutelevageventedirecte", "bande_id", "INTEGER"),
        ("coutelevageventedirecte", "nb_porcs_calcules", "INTEGER"),
        ("coutelevageventedirecte", "poids_moyen_kg", "REAL"),
        ("coutelevageventedirecte", "cout_par_porc", "REAL"),
        ("coutelevageventedirecte", "cout_par_kg", "REAL"),
        ("coutelevageventedirecte", "calcule_le", "TEXT"),
        (
            "reglageventedirecte",
            "commandes_ouvertes",
            "INTEGER NOT NULL DEFAULT 1",
        ),
        ("reglageventedirecte", "message_fermeture", "TEXT"),
        ("reglageventedirecte", "logo_data", "BLOB"),
        ("reglageventedirecte", "logo_mime", "TEXT"),
        (
            "releve_compteur",
            "remplacement_compteur",
            "INTEGER NOT NULL DEFAULT 0",
        ),
        ("releve_compteur", "prix_unitaire", "REAL"),
        // Traçabilité reportée depuis le code scanné sur l'emballage : le lot
        // et la péremption doivent suivre le mouvement, pas le produit, car
        // ils changent d'une boîte à l'autre.
        ("mouvementpharmacie", "lot", "TEXT"),
        ("mouvementpharmacie", "peremption", "TEXT"),
        ("mouvementpharmacie", "code_scanne", "TEXT"),
        ("importjournal", "contenu_sha256", "TEXT"),
        ("venteapport", "montant_ht", "REAL"),
        ("evenement", "creneaux_ia", "TEXT"),
        ("evenement", "case_id", "INTEGER"),
        ("site", "zone", "TEXT"),
        ("casesalle", "nb_max_truies", "INTEGER"),
        ("casesalle", "nb_max_porcelets", "INTEGER"),
        ("evenement", "date_fin", "TEXT"),
        // Dimensions de salle : servent au calcul du m²/m³ et, avec le débit
        // des ventilateurs déclarés (0011), au taux de renouvellement d'air.
        ("salle", "longueur", "REAL"),
        ("salle", "largeur", "REAL"),
        ("salle", "hauteur", "REAL"),
        ("salle", "mode_chauffage", "TEXT"),
        // Formule de conversion (ex. sonde TC100) pour une entrée analogique :
        // l'emporte sur coefficient/décalage quand elle est renseignée.
        ("ipx_point", "formule", "TEXT"),
        // Position de la salle sur le plan visuel du site (voir
        // /structure/plan) : coordonnées et taille dans un repère propre à
        // chaque élevage, saisies une fois par un administrateur dans
        // l'éditeur glisser-déposer. NULL tant que la salle n'a pas encore
        // été placée — elle apparaît alors dans la liste "à placer".
        ("salle", "plan_x", "REAL"),
        ("salle", "plan_y", "REAL"),
        ("salle", "plan_largeur", "REAL"),
        ("salle", "plan_hauteur", "REAL"),
        ("salle", "plan_rotation", "REAL NOT NULL DEFAULT 0"),
        // Dernier transfert connu à la saisie de l'inventaire : les
        // transferts du même jour enregistrés après lui sont comptés.
        ("inventairecase", "dernier_transfert_id", "INTEGER"),
    ] {
        ensure_column(pool, table, column, definition).await?;
    }
    // Ventilateurs déclarés par salle : position, fonction, voie et débit,
    // pour dimensionner la ventilation indépendamment de la régulation elle-même.
    sqlx::raw_sql(include_str!("../migrations/0011_ventilation_salle.sql"))
        .execute(pool)
        .await?;
    // Observations comportementales (cannibalisme, queue/oreille mordue...)
    // saisies depuis la saisie rapide, pour les croiser ensuite avec le
    // climat de la salle au même moment.
    sqlx::raw_sql(include_str!(
        "../migrations/0012_observation_comportement.sql"
    ))
    .execute(pool)
    .await?;
    // Préconditionnement : hypothèse d'arrivée future d'une bande dans une
    // salle, pour démarrer un préchauffage avant le transfert réel.
    sqlx::raw_sql(include_str!("../migrations/0013_preconditionnement.sql"))
        .execute(pool)
        .await?;
    // Plan du site (voir /structure/plan) : image de fond et blocs libres.
    sqlx::raw_sql(include_str!("../migrations/0015_plan.sql"))
        .execute(pool)
        .await?;
    // Programmes d'éclairage (voir /ambiance/eclairage), appliqués par eo-regul.
    sqlx::raw_sql(include_str!("../migrations/0016_eclairage.sql"))
        .execute(pool)
        .await?;
    // Réseau Zigbee (voir /ambiance/zigbee), alimenté par le pont d'eo-regul.
    sqlx::raw_sql(include_str!("../migrations/0017_zigbee.sql"))
        .execute(pool)
        .await?;
    // Surveillance électrique des appareils (compteurs Zigbee).
    sqlx::raw_sql(include_str!("../migrations/0018_surveillance_electrique.sql"))
        .execute(pool)
        .await?;
    // Prestataires : bandes confiées, aliment, pesées (voir /engraissement).
    sqlx::raw_sql(include_str!("../migrations/0019_prestataires.sql"))
        .execute(pool)
        .await?;
    // Alertes SMS Free Mobile (voir /equipements/sms).
    sqlx::raw_sql(include_str!("../migrations/0020_sms.sql"))
        .execute(pool)
        .await?;
    // Travail de la semaine pointé truie par truie (voir /conduite/semaine).
    sqlx::raw_sql(include_str!("../migrations/0021_travail_fait.sql"))
        .execute(pool)
        .await?;
    // Démarrages des machines (détecteurs de vibration Zigbee).
    sqlx::raw_sql(include_str!("../migrations/0022_demarrages.sql"))
        .execute(pool)
        .await?;
    // Caméras des salles (voir /equipements/cameras).
    sqlx::raw_sql(include_str!("../migrations/0023_camera.sql"))
        .execute(pool)
        .await?;
    // Machine à soupe Asserva suivie en écoute (voir /equipements/soupe).
    sqlx::raw_sql(include_str!("../migrations/0024_soupe.sql"))
        .execute(pool)
        .await?;
    // Départs de porcs prévus (voir /departs).
    sqlx::raw_sql(include_str!("../migrations/0025_depart_prevu.sql"))
        .execute(pool)
        .await?;
    // Consommation électrique par jour (voir routes::electricite).
    sqlx::raw_sql(include_str!("../migrations/0026_electricite.sql"))
        .execute(pool)
        .await?;
    sqlx::raw_sql(include_str!("../migrations/0027_elec_heure.sql"))
        .execute(pool)
        .await?;
    // Module Automatique : règles si… alors… (voir routes::automatique).
    sqlx::raw_sql(include_str!("../migrations/0028_automatique.sql"))
        .execute(pool)
        .await?;
    // Repères du plan de l'élevage (voir routes::plan_elevage).
    sqlx::raw_sql(include_str!("../migrations/0029_plan_repere.sql"))
        .execute(pool)
        .await?;
    // Suivi des cours sur la page Quotidien (voir cours.rs).
    sqlx::raw_sql(include_str!("../migrations/0030_cours.sql"))
        .execute(pool)
        .await?;
    // Ventilation d'une salle vide (économie) : NULL = réglage désactivé.
    ensure_column(pool, "ambiance_salle", "ventil_vide", "REAL").await?;
    // Plan de l'élevage pivoté par quarts de tour (voir routes::plan_elevage).
    ensure_column(pool, "plan_reglage", "rotation", "INTEGER NOT NULL DEFAULT 0").await?;
    // Taille d'un encart du plan, en % du plan (NULL = taille du contenu).
    ensure_column(pool, "plan_repere", "w", "REAL").await?;
    ensure_column(pool, "plan_repere", "h", "REAL").await?;
    // Classement par nom des objets de l'automate : dépend de la colonne
    // `motif` ajoutée juste au-dessus.
    sqlx::raw_sql(include_str!("../migrations/0010_ipx_section_motif.sql"))
        .execute(pool)
        .await?;
    // Remplacer uniquement la vue de calcul, atomiquement, après les colonnes additives.
    let mut tx = pool.begin().await?;
    sqlx::raw_sql(include_str!("../migrations/0003_portee_effectif.sql"))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    // Après les colonnes additives : compatible avec les bases anciennes.
    sqlx::raw_sql(include_str!("../migrations/0002_ventelot.sql"))
        .execute(pool)
        .await?;
    sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS ux_importjournal_contenu_sha256 ON importjournal(contenu_sha256) WHERE contenu_sha256 IS NOT NULL")
        .execute(pool)
        .await?;

    sqlx::raw_sql("CREATE INDEX IF NOT EXISTS quotidien_date_id ON controlequotidien(date,id); CREATE INDEX IF NOT EXISTS quotidien_notes_date ON controlequotidien(categorie,date,id);").execute(pool).await?;
    Ok(())
}

/// Suite de `init` après les migrations : réparations de données, comptes et
/// nettoyages historiques. Jamais appelée seule sur une base de contrôle
/// (voir `etablir_schema`) : ce n'est pas du schéma, c'est de la donnée.
async fn achever_init(pool: &SqlitePool) -> anyhow::Result<()> {
    // Transferts de porcs de case à case enregistrés sans leur bande (avant
    // la 2.26.2) : la bande est celle de la case de départ à cette date.
    // Idempotent, et ne touche que les transferts dont l'origine est connue.
    sqlx::query(
        "UPDATE transfert SET bande_id=(SELECT p.bande_id FROM transfert p \
           WHERE p.case_dest_id=transfert.case_source_id AND p.bande_id IS NOT NULL AND p.espece='porc' \
             AND p.date<=transfert.date ORDER BY p.date DESC, p.id DESC LIMIT 1) \
         WHERE espece='porc' AND bande_id IS NULL AND case_source_id IS NOT NULL AND truie_id IS NULL \
           AND vente_apport_id IS NULL AND EXISTS(SELECT 1 FROM transfert p WHERE p.case_dest_id=transfert.case_source_id \
             AND p.bande_id IS NOT NULL AND p.espece='porc' AND p.date<=transfert.date)",
    )
    .execute(pool)
    .await?;
    // Les anciens numéros libres deviennent immédiatement des choix fiables.
    sqlx::query("INSERT OR IGNORE INTO numeromarquage(numero) SELECT DISTINCT upper(trim(num_officiel)) FROM bande WHERE num_officiel IS NOT NULL AND trim(num_officiel)<>''")
        .execute(pool)
        .await?;
    sqlx::query("UPDATE bande SET num_officiel=upper(trim(num_officiel)) WHERE num_officiel IS NOT NULL AND trim(num_officiel)<>''")
        .execute(pool)
        .await?;

    // Prépare également les soins des portées déjà présentes lors d'une mise
    // à jour depuis une ancienne version. L'unicité évite tout doublon.
    sqlx::query("INSERT OR IGNORE INTO soinportee(protocole_id,evenement_id,truie_id,bande_id,date_prevue) SELECT a.id,e.id,e.truie_id,e.bande_id,date(e.date,printf('%+d day',a.jour)) FROM evenement e JOIN acteprotocole a ON a.actif=1 AND lower(trim(a.cible)) IN ('porcelet','porcelets') AND lower(trim(a.reference))='mise_bas' WHERE e.type='mise_bas' AND e.truie_id IS NOT NULL")
        .execute(pool)
        .await?;

    // Reprise 2.2.29 : les anciennes bases ne possédaient pas la colonne
    // `montant_ht` sur les ventes. Pour les apports Cooperl, le HT fiable est
    // déjà conservé dans le JSON de chaque lot. On ne reconstitue jamais un HT
    // depuis le net global : les lignes sans source HT restent volontairement
    // à NULL et sont signalées dans l'interface.
    sqlx::query(
        "UPDATE venteapport SET montant_ht=ROUND((SELECT SUM(CAST(json_extract(j.value,'$.montant_ht') AS REAL)) FROM json_each(venteapport.lots_json) j),2) WHERE montant_ht IS NULL AND json_valid(lots_json) AND json_type(lots_json)='array' AND EXISTS(SELECT 1 FROM json_each(venteapport.lots_json) j WHERE json_extract(j.value,'$.montant_ht') IS NOT NULL)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE venteapport SET montant_ht=ROUND(CAST(json_extract(lots_json,'$.montant_ht') AS REAL),2) WHERE montant_ht IS NULL AND json_valid(lots_json) AND json_type(lots_json)='object' AND json_extract(lots_json,'$.montant_ht') IS NOT NULL",
    )
    .execute(pool)
    .await?;

    sqlx::query("UPDATE commandeventedirecte SET token_modification=lower(hex(randomblob(32))) WHERE token_modification IS NULL OR trim(token_modification)=''")
        .execute(pool)
        .await?;
    sqlx::query("UPDATE commandeventedirecte SET code_modification=upper(substr(hex(randomblob(8)),1,8)) WHERE code_modification IS NULL OR trim(code_modification)=''")
        .execute(pool)
        .await?;

    sqlx::query("CREATE TRIGGER IF NOT EXISTS repartition_verrat_nouvelle_bande AFTER INSERT ON bande BEGIN INSERT OR IGNORE INTO affectationfacturebande(categorie,facture_id,bande_id,automatique) SELECT 'genetique',id,NEW.id,1 FROM achatgenetique WHERE toutes_bandes=1; END").execute(pool).await?;
    // Facture de verrat explicitement identifiée par l'éleveur, sans modifier le HT.
    sqlx::query("UPDATE achatgenetique SET toutes_bandes=1,designation='1 verrat — Vigor souffleur',nb_animaux=1 WHERE replace(num_facture,'.','')='1441384' AND date='2025-07-08' AND COALESCE(designation,'') NOT LIKE '%verrat%' AND NOT EXISTS(SELECT 1 FROM affectationfacturecontrole c WHERE c.categorie='genetique' AND c.facture_id=achatgenetique.id AND c.verrou_manuel=1)").execute(pool).await?;
    repair_genetic_ht(pool).await?;
    auto_assign_economic_invoices(pool).await?;

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM utilisateur")
        .fetch_one(pool)
        .await?;
    if count == 0 {
        let hash = auth::hash_password_async("admin".to_string()).await?;
        sqlx::query(
            "INSERT INTO utilisateur(identifiant,nom,hash_mdp,role,actif,doit_changer_mdp) VALUES('admin','Administrateur',?,'admin',1,1)",
        )
        .bind(hash)
        .execute(pool)
        .await?;
        tracing::warn!("Compte admin initial créé; mot de passe temporaire: admin");
    }
    Ok(())
}

/// Reprend les anciennes affectations puis propose une bande aux factures qui
/// n'en ont aucune. Une correction manuelle, y compris « aucune bande », est
/// verrouillée et ne sera jamais écrasée au prochain démarrage.
pub async fn auto_assign_economic_invoices(pool: &SqlitePool) -> anyhow::Result<u64> {
    let mut changed = crate::affectation::boars(pool).await?;
    for (category, table) in [("aliment", "livraisonaliment"), ("veto", "achatveto")] {
        let sql = format!("INSERT OR IGNORE INTO affectationfacturebande(categorie,facture_id,bande_id,automatique) SELECT '{category}',x.id,CAST(j.value AS INTEGER),0 FROM {table} x,json_each('['||x.bandes||']') j WHERE x.bandes IS NOT NULL AND NOT EXISTS(SELECT 1 FROM affectationfacturebande a WHERE a.categorie='{category}' AND a.facture_id=x.id) AND NOT EXISTS(SELECT 1 FROM affectationfacturecontrole c WHERE c.categorie='{category}' AND c.facture_id=x.id) AND json_valid('['||x.bandes||']') AND EXISTS(SELECT 1 FROM bande b WHERE b.id=CAST(j.value AS INTEGER))");
        changed += sqlx::query(&sql).execute(pool).await?.rows_affected();
    }

    for (category, table) in [
        ("aliment", "livraisonaliment"),
        ("veto", "achatveto"),
        ("semence", "achatsemence"),
    ] {
        let sql = format!(
            "INSERT OR IGNORE INTO affectationfacturebande(categorie,facture_id,bande_id,automatique) SELECT '{category}',id,bande_id,0 FROM {table} x WHERE bande_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM affectationfacturebande a WHERE a.categorie='{category}' AND a.facture_id=x.id) AND NOT EXISTS(SELECT 1 FROM affectationfacturecontrole c WHERE c.categorie='{category}' AND c.facture_id=x.id)"
        );
        changed += sqlx::query(&sql).execute(pool).await?.rows_affected();
    }
    changed += sqlx::query("INSERT OR IGNORE INTO affectationfacturebande(categorie,facture_id,bande_id,automatique) SELECT 'genetique',a.id,b.id,0 FROM achatgenetique a JOIN bande b ON b.code=a.bande_code WHERE a.toutes_bandes=0 AND a.bande_code IS NOT NULL AND trim(a.bande_code)<>''")
        .execute(pool).await?.rows_affected();

    for (category, table, center, has_site) in [
        ("semence", "achatsemence", -114_i64, false),
        ("genetique", "achatgenetique", -180_i64, false),
    ] {
        let site_order = if has_site {
            "CASE WHEN trim(COALESCE(x.site,''))='' THEN 1 WHEN lower(trim(COALESCE(b.site,'')))=lower(trim(x.site)) THEN 0 ELSE 2 END,"
        } else {
            ""
        };
        let sql = format!(
            "WITH candidats AS (SELECT x.id AS facture_id,b.id AS bande_id,ABS((julianday(x.date)-julianday(b.date_mb))-({center})) AS score,ROW_NUMBER() OVER(PARTITION BY x.id ORDER BY {site_order} CASE WHEN x.date IS NULL THEN CASE WHEN b.active=1 THEN 0 ELSE 1 END ELSE ABS((julianday(x.date)-julianday(b.date_mb))-({center})) END,b.active DESC,b.date_mb DESC,b.id DESC) AS rang FROM {table} x CROSS JOIN bande b WHERE b.date_mb IS NOT NULL AND NOT EXISTS(SELECT 1 FROM affectationfacturebande a WHERE a.categorie='{category}' AND a.facture_id=x.id) AND NOT EXISTS(SELECT 1 FROM affectationfacturecontrole c WHERE c.categorie='{category}' AND c.facture_id=x.id AND c.verrou_manuel=1)) INSERT OR IGNORE INTO affectationfacturebande(categorie,facture_id,bande_id,automatique,score) SELECT '{category}',facture_id,bande_id,1,score FROM candidats WHERE rang=1"
        );
        changed += sqlx::query(&sql).execute(pool).await?.rows_affected();
    }

    changed += crate::affectation::refresh(pool, "aliment", "livraisonaliment").await?;
    changed += crate::affectation::refresh(pool, "veto", "achatveto").await?;
    sqlx::query("UPDATE livraisonaliment SET bande_id=(SELECT a.bande_id FROM affectationfacturebande a WHERE a.categorie='aliment' AND a.facture_id=livraisonaliment.id ORDER BY a.id LIMIT 1),bandes=(SELECT GROUP_CONCAT(a.bande_id) FROM affectationfacturebande a WHERE a.categorie='aliment' AND a.facture_id=livraisonaliment.id)").execute(pool).await?;
    sqlx::query("UPDATE achatveto SET bande_id=(SELECT a.bande_id FROM affectationfacturebande a WHERE a.categorie='veto' AND a.facture_id=achatveto.id ORDER BY a.id LIMIT 1),bandes=(SELECT GROUP_CONCAT(a.bande_id) FROM affectationfacturebande a WHERE a.categorie='veto' AND a.facture_id=achatveto.id)").execute(pool).await?;
    sqlx::query("UPDATE achatsemence SET bande_id=(SELECT a.bande_id FROM affectationfacturebande a WHERE a.categorie='semence' AND a.facture_id=achatsemence.id ORDER BY a.id LIMIT 1)").execute(pool).await?;
    sqlx::query("UPDATE achatgenetique SET bande_code=(SELECT b.code FROM affectationfacturebande a JOIN bande b ON b.id=a.bande_id WHERE a.categorie='genetique' AND a.facture_id=achatgenetique.id ORDER BY a.id LIMIT 1)").execute(pool).await?;
    Ok(changed)
}

/// Ne répare que les valeurs TTC reconnaissables dans une source importée
/// appliquée. Aucun taux de TVA supposé, aucune correction manuelle écrasée.
pub async fn repair_genetic_ht(pool: &SqlitePool) -> anyhow::Result<u64> {
    let rows: Vec<(i64, Option<String>, Option<f64>)> =
        sqlx::query_as("SELECT id,num_facture,montant_ht FROM achatgenetique WHERE ht_manuel=0")
            .fetch_all(pool)
            .await?;
    let sources:Vec<String>=sqlx::query_scalar("SELECT l.donnees_json FROM importligne l JOIN importjournal j ON j.token=l.token WHERE j.statut='applique' AND j.type_import LIKE 'economique:%'").fetch_all(pool).await?;
    let mut changed = 0;
    for (id, reference, current) in rows {
        if reference.as_deref().is_none_or(|r| r.trim().is_empty()) {
            continue;
        }
        let mut candidates = Vec::new();
        for raw in &sources {
            let Ok(line) = serde_json::from_str::<crate::economic_import::ImportLine>(raw) else {
                continue;
            };
            if line.kind != "genetique" || line.reference != reference {
                continue;
            }
            if let Some(ht) = line.details["montant_ht"]
                .as_f64()
                .filter(|x| x.is_finite())
            {
                if line.amount == Some(ht)
                    && (current.is_none()
                        || (line.details["montant_net"].as_f64() == current && current != Some(ht)))
                {
                    candidates.push(ht);
                }
            }
        }
        if let Some(ht) = candidates
            .first()
            .copied()
            .filter(|ht| candidates.iter().all(|x| x == ht))
        {
            changed +=
                sqlx::query("UPDATE achatgenetique SET montant_ht=? WHERE id=? AND ht_manuel=0")
                    .bind(ht)
                    .bind(id)
                    .execute(pool)
                    .await?
                    .rows_affected();
        }
    }
    Ok(changed)
}

async fn verify_sqlite_pragmas(pool: &SqlitePool) -> anyhow::Result<()> {
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(pool)
        .await?;
    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout")
        .fetch_one(pool)
        .await?;
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(pool)
        .await?;
    anyhow::ensure!(
        journal_mode.eq_ignore_ascii_case("wal"),
        "SQLite doit fonctionner en WAL (mode obtenu: {journal_mode})"
    );
    anyhow::ensure!(
        busy_timeout >= 5_000,
        "SQLite busy_timeout inférieur à 5000 ms"
    );
    anyhow::ensure!(
        foreign_keys == 1,
        "les clés étrangères SQLite sont désactivées"
    );
    Ok(())
}

async fn ensure_column(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    definition: &str,
) -> anyhow::Result<()> {
    let sql = format!("PRAGMA table_info(\"{}\")", table.replace('"', ""));
    let rows = sqlx::query(&sql).fetch_all(pool).await?;
    if rows
        .iter()
        .any(|row| row.get::<String, _>("name") == column)
    {
        return Ok(());
    }
    let alter = format!(
        "ALTER TABLE \"{}\" ADD COLUMN \"{}\" {}",
        table.replace('"', ""),
        column.replace('"', ""),
        definition
    );
    sqlx::query(&alter).execute(pool).await?;
    Ok(())
}

pub async fn journal(
    pool: &SqlitePool,
    utilisateur: &str,
    action: &str,
    objet: &str,
    detail: &str,
    chemin: &str,
) {
    if let Err(error) =
        sqlx::query("INSERT INTO journal(horodatage,utilisateur,action,objet,detail,chemin) VALUES(CURRENT_TIMESTAMP,?,?,?,?,?)")
            .bind(utilisateur)
            .bind(action)
            .bind(objet)
            .bind(detail)
            .bind(chemin)
            .execute(pool)
            .await
    {
        tracing::warn!(%error, "journalisation impossible");
    }
}

#[cfg(test)]
mod economic_assignment_tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn propose_la_bande_probable_et_respecte_un_retrait_manuel() -> anyhow::Result<()> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        sqlx::raw_sql(include_str!("../migrations/0001_schema.sql"))
            .execute(&pool)
            .await?;
        let first =
            sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('B1','2026-01-01',1)")
                .execute(&pool)
                .await?
                .last_insert_rowid();
        let second =
            sqlx::query("INSERT INTO bande(code,date_mb,active) VALUES('B2','2026-04-01',1)")
                .execute(&pool)
                .await?
                .last_insert_rowid();
        sqlx::raw_sql("INSERT INTO site(id,code) VALUES(1,'S1'); INSERT INTO salle(id,site_id,nom) VALUES(1,1,'S1'); INSERT INTO transfert(date,bande_id,salle_dest_id,nombre) VALUES('2026-02-01',1,1,10),('2026-04-01',2,1,10);").execute(&pool).await?;
        let invoice = sqlx::query("INSERT INTO livraisonaliment(date,produit,montant_ht,sites_json,stade_aliment) VALUES('2026-03-12','Aliment test',1000,'[1]','tous')")
            .execute(&pool).await?.last_insert_rowid();
        let removed = sqlx::query("INSERT INTO livraisonaliment(date,produit,montant_ht) VALUES('2026-03-12','Retrait manuel',500)")
            .execute(&pool).await?.last_insert_rowid();
        sqlx::query("INSERT INTO affectationfacturecontrole(categorie,facture_id,verrou_manuel) VALUES('aliment',?,1)")
            .bind(removed).execute(&pool).await?;

        auto_assign_economic_invoices(&pool).await?;
        let proposed: i64 = sqlx::query_scalar("SELECT bande_id FROM affectationfacturebande WHERE categorie='aliment' AND facture_id=?")
            .bind(invoice).fetch_one(&pool).await?;
        let removed_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM affectationfacturebande WHERE categorie='aliment' AND facture_id=?")
            .bind(removed).fetch_one(&pool).await?;
        assert_eq!(proposed, first);
        assert_eq!(removed_count, 0);
        sqlx::query("INSERT INTO affectationfacturebande(categorie,facture_id,bande_id,automatique) VALUES('aliment',?,?,0)")
            .bind(invoice).bind(second).execute(&pool).await?;
        let distributed_total: f64 = sqlx::query_scalar("SELECT CAST(SUM(x.montant_ht/(SELECT COUNT(*) FROM affectationfacturebande n WHERE n.categorie='aliment' AND n.facture_id=x.id)) AS REAL) FROM livraisonaliment x JOIN affectationfacturebande a ON a.categorie='aliment' AND a.facture_id=x.id WHERE x.id=?")
            .bind(invoice).fetch_one(&pool).await?;
        assert!((distributed_total - 1000.0).abs() < 0.001);
        Ok(())
    }
}

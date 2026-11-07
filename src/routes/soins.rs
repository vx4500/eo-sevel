//! Page « Soins » : tout ce qui touche aux soins sur une seule page.
//! 1. À faire (plan sanitaire), 2. Enregistrer un soin, 3. Pharmacie,
//! 4. Derniers soins. Chaque soin sort ses doses du stock pharmacie.

use super::*;

/// Motifs proposés (texte libre accepté).
pub(super) const MOTIFS: &[&str] = &[
    "Boiterie",
    "Mammite",
    "Digestif",
    "Respiratoire",
    "Fièvre",
    "Mise-bas difficile",
    "Synchronisation des chaleurs",
    "Vaccin",
    "Déparasitage",
    "Autre",
];

/// Préfixe des notes de sortie de stock d'un soin de bande (sert à les
/// retrouver dans l'historique et à les annuler).
const NOTE_BANDE: &str = "Soin de bande";

pub(super) const PRODUITS: &str = "WITH noms AS (\
    SELECT produit FROM produitpharmacie \
    UNION ALL SELECT produit FROM evenement WHERE type='traitement' AND produit IS NOT NULL \
    UNION ALL SELECT produit FROM traitementcharcutier WHERE produit IS NOT NULL \
    UNION ALL SELECT produit FROM acteprotocole WHERE actif=1 AND produit IS NOT NULL), \
  n AS (SELECT MIN(trim(produit)) AS produit FROM noms WHERE trim(produit)<>'' GROUP BY lower(trim(produit))) \
  SELECT n.produit,p.id,CASE WHEN COALESCE(p.stock_actuel,0)=CAST(COALESCE(p.stock_actuel,0) AS INTEGER) THEN CAST(COALESCE(p.stock_actuel,0) AS INTEGER) ELSE ROUND(p.stock_actuel,2) END AS stock,COALESCE(p.unite,'doses') AS unite,p.seuil_alerte,p.note, \
  CASE WHEN p.id IS NOT NULL AND (COALESCE(p.stock_actuel,0)<0 OR (p.seuil_alerte IS NOT NULL AND p.stock_actuel<=p.seuil_alerte)) THEN 1 ELSE 0 END AS alerte, \
  (SELECT MAX(d) FROM (SELECT date AS d FROM evenement e WHERE e.type='traitement' AND lower(e.produit)=lower(n.produit) \
     UNION ALL SELECT date FROM mouvementpharmacie m WHERE m.type='sortie' AND lower(m.produit)=lower(n.produit) \
     UNION ALL SELECT date FROM traitementcharcutier tc WHERE lower(tc.produit)=lower(n.produit))) AS dernier, \
  (SELECT e.nb_doses FROM evenement e WHERE e.type='traitement' AND lower(e.produit)=lower(n.produit) AND e.nb_doses>0 ORDER BY e.date DESC,e.id DESC LIMIT 1) AS doses, \
  COALESCE((SELECT e.delai_attente FROM evenement e WHERE e.type='traitement' AND lower(e.produit)=lower(n.produit) AND e.delai_attente IS NOT NULL ORDER BY e.date DESC,e.id DESC LIMIT 1), \
     (SELECT a.delai_attente FROM acteprotocole a WHERE lower(a.produit)=lower(n.produit) AND a.delai_attente IS NOT NULL LIMIT 1)) AS delai, \
  (SELECT e.motif FROM evenement e WHERE e.type='traitement' AND lower(e.produit)=lower(n.produit) AND e.motif IS NOT NULL ORDER BY e.date DESC,e.id DESC LIMIT 1) AS motif \
  FROM n LEFT JOIN produitpharmacie p ON lower(p.produit)=lower(n.produit) \
  ORDER BY dernier IS NULL,dernier DESC,n.produit";

/// Derniers soins, tous animaux confondus.
const HISTORIQUE: &str = "SELECT * FROM (\
  SELECT e.date,'truie' AS qui,'Truie '||t.num_travail AS animal,'/truie/'||t.id||'#soins' AS lien,e.produit,e.nb_doses AS doses,e.date_fin,e.motif,e.delai_attente,e.note,NULL AS annuler,e.id AS tri \
    FROM evenement e JOIN truie t ON t.id=e.truie_id WHERE e.type='traitement' \
  UNION ALL SELECT e.date,'verrat','Verrat '||v.code,'/verrat/'||v.id,e.produit,e.nb_doses,e.date_fin,e.motif,e.delai_attente,e.note,NULL,e.id \
    FROM evenement e JOIN verrat v ON v.id=e.verrat_id WHERE e.type='traitement' AND e.truie_id IS NULL \
  UNION ALL SELECT tc.date,'porc','Porc '||COALESCE(NULLIF(p.rfid,''),'#'||p.id),'/charcutier/'||p.id,tc.produit,tc.dose,NULL,tc.motif,tc.delai_attente,tc.note,NULL,tc.id \
    FROM traitementcharcutier tc JOIN porccharcutier p ON p.id=tc.charcutier_id \
  UNION ALL SELECT m.date,'bande','Bande '||COALESCE(m.bande_code,'—'),NULL,m.produit,m.quantite,NULL,NULL,NULL,substr(m.note,length('Soin de bande')+4),m.id,m.id \
    FROM mouvementpharmacie m WHERE m.type='sortie' AND m.note LIKE 'Soin de bande%' \
  UNION ALL SELECT ar.date_realise,'bande','Bande '||b.code,'/bande/'||b.id,a.produit,NULL,NULL,'Plan sanitaire : '||a.libelle,a.delai_attente,ar.note,NULL,ar.id \
    FROM acterealise ar JOIN acteprotocole a ON a.id=ar.acte_id JOIN bande b ON b.id=ar.bande_id \
  ) ORDER BY date DESC,tri DESC LIMIT 200";

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let mut ctx = context(&session);
    ctx.insert("a_faire".into(), Value::Array(conduite::soins_a_faire(pool, 7, 30).await?));
    ctx.insert("produits".into(), Value::Array(generic_rows(pool, PRODUITS).await?));
    ctx.insert("truies".into(), Value::Array(generic_rows(pool, "SELECT id,num_travail AS code FROM truie WHERE reformee=0 ORDER BY CAST(num_travail AS INTEGER),num_travail").await?));
    ctx.insert("bandes".into(), Value::Array(generic_rows(pool, "SELECT id,code FROM bande WHERE active=1 ORDER BY date_mb DESC,code").await?));
    ctx.insert("verrats".into(), Value::Array(generic_rows(pool, "SELECT id,code FROM verrat WHERE actif=1 ORDER BY code").await?));
    ctx.insert("historique".into(), Value::Array(generic_rows(pool, HISTORIQUE).await?));
    ctx.insert("motifs".into(), json!(MOTIFS));
    ctx.insert("gere_stock".into(), json!(matches!(session.role.as_str(), "admin" | "eleveur")));
    ctx.insert("ok".into(), json!(query.get("ok")));
    ctx.insert("today".into(), json!(today_iso()));
    render(&state, "soins.html", Value::Object(ctx))
}

fn texte_soin(motif: Option<&str>, doses: i64, jours: i64, delai: Option<i64>, note: Option<&str>) -> String {
    let mut parts = Vec::new();
    if let Some(motif) = motif {
        parts.push(motif.to_string());
    }
    parts.push(if jours > 1 {
        format!("{doses} dose(s) par jour pendant {jours} jours")
    } else {
        format!("{doses} dose(s)")
    });
    if let Some(delai) = delai {
        parts.push(format!("attente {delai} j"));
    }
    if let Some(note) = note {
        parts.push(note.to_string());
    }
    parts.join(" · ")
}

fn retour(form: &HashMap<String, String>, defaut: &str) -> String {
    form_text(form, "retour")
        .filter(|r| r.starts_with('/') && !r.starts_with("//"))
        .unwrap_or_else(|| defaut.to_string())
}

/// Un seul enregistrement pour tous les animaux : qui, quel produit,
/// combien, pourquoi.
pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let qui = if form_text(&form, "truie_id").is_some() {
        "truie".to_string()
    } else {
        form_text(&form, "qui").unwrap_or_default()
    };
    let numero = form_text(&form, "numero");
    let produit = form_text(&form, "produit")
        .ok_or_else(|| AppError::Invalid("Choisissez le produit donné".into()))?;
    let date = form_date_or_today(&form, "date")?;
    let start = parse_stored_date(&date).ok_or_else(|| AppError::Invalid("Date invalide".into()))?;
    let entier = |cle: &str, nom: &str| -> AppResult<i64> {
        match form_text(&form, cle) {
            None => Ok(1),
            Some(v) => v
                .parse::<i64>()
                .ok()
                .filter(|n| (1..=1000).contains(n))
                .ok_or_else(|| AppError::Invalid(format!("{nom} : un nombre entier, 1 ou plus"))),
        }
    };
    let doses = entier("nb_doses", "Doses")?;
    let jours = entier("jours", "Nombre de jours")?;
    if jours > 365 {
        return Err(AppError::Invalid("Nombre de jours : 365 au plus".into()));
    }
    let date_fin = (jours > 1).then(|| (start + Duration::days(jours - 1)).to_string());
    let motif = form_text(&form, "motif");
    let delai = form_i64(&form, "delai_attente").filter(|d| *d >= 0);
    let note = form_text(&form, "note");
    let introuvable = |quoi: &str| {
        AppError::Invalid(format!(
            "{quoi} {} introuvable",
            numero.as_deref().unwrap_or("(numéro vide)")
        ))
    };
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let cible = match qui.as_str() {
        "truie" => {
            let id: i64 = match form_i64(&form, "truie_id") {
                Some(id) => sqlx::query_scalar("SELECT id FROM truie WHERE id=?").bind(id).fetch_optional(&mut *tx).await?,
                None => sqlx::query_scalar("SELECT id FROM truie WHERE num_travail=? ORDER BY reformee,id DESC LIMIT 1")
                    .bind(&numero).fetch_optional(&mut *tx).await?,
            }
            .ok_or_else(|| introuvable("Truie"))?;
            let bande: Option<i64> = sqlx::query_scalar("SELECT b.id FROM truie t JOIN bande b ON b.code=t.bande_code WHERE t.id=? ORDER BY b.active DESC,b.id DESC LIMIT 1")
                .bind(id).fetch_optional(&mut *tx).await?;
            let event = sqlx::query("INSERT INTO evenement(type,date,date_fin,truie_id,bande_id,produit,motif,delai_attente,nb_doses,note,suivi_actif) VALUES('traitement',?,?,?,?,?,?,?,?,?,0)")
                .bind(&date).bind(&date_fin).bind(id).bind(bande).bind(&produit).bind(&motif).bind(delai).bind(doses).bind(&note)
                .execute(&mut *tx).await?.last_insert_rowid();
            traitement_stock::appliquer(&mut tx, event).await?;
            format!("truie #{id}")
        }
        "verrat" => {
            let id: i64 = sqlx::query_scalar("SELECT id FROM verrat WHERE code=? ORDER BY actif DESC,id DESC LIMIT 1")
                .bind(&numero).fetch_optional(&mut *tx).await?.ok_or_else(|| introuvable("Verrat"))?;
            let event = sqlx::query("INSERT INTO evenement(type,date,date_fin,verrat_id,produit,motif,delai_attente,nb_doses,note,suivi_actif) VALUES('traitement',?,?,?,?,?,?,?,?,0)")
                .bind(&date).bind(&date_fin).bind(id).bind(&produit).bind(&motif).bind(delai).bind(doses).bind(&note)
                .execute(&mut *tx).await?.last_insert_rowid();
            traitement_stock::appliquer(&mut tx, event).await?;
            format!("verrat #{id}")
        }
        "porc" => {
            let saisi = numero.clone().unwrap_or_default();
            let interne = saisi.trim_start_matches('#').parse::<i64>().ok();
            let porc: Option<(i64, Option<String>)> = sqlx::query_as(
                "SELECT id,bande_code FROM porccharcutier WHERE rfid=? OR (? IS NOT NULL AND id=? AND ? LIKE '#%') ORDER BY id DESC LIMIT 1",
            )
            .bind(&saisi).bind(interne).bind(interne).bind(&saisi)
            .fetch_optional(&mut *tx).await?;
            let (id, bande) = porc.ok_or_else(|| introuvable("Porc"))?;
            let dose_txt = if jours > 1 { format!("{doses} dose(s) par jour pendant {jours} jours") } else { format!("{doses} dose(s)") };
            let tc = sqlx::query("INSERT INTO traitementcharcutier(charcutier_id,bande_code,date,produit,dose,motif,delai_attente,note) VALUES(?,?,?,?,?,?,?,?)")
                .bind(id).bind(&bande).bind(&date).bind(&produit).bind(dose_txt).bind(&motif).bind(delai).bind(&note)
                .execute(&mut *tx).await?.last_insert_rowid();
            traitement_stock::sortie(
                &mut tx, &produit, &date, (doses * jours) as f64,
                &format!("Soin porc #{tc} : {}", texte_soin(motif.as_deref(), doses, jours, delai, note.as_deref())),
                bande.as_deref(),
            ).await?;
            format!("porc #{id}")
        }
        "bande" => {
            let code: String = sqlx::query_scalar("SELECT code FROM bande WHERE code=? ORDER BY active DESC,id DESC LIMIT 1")
                .bind(&numero).fetch_optional(&mut *tx).await?.ok_or_else(|| introuvable("Bande"))?;
            traitement_stock::sortie(
                &mut tx, &produit, &date, (doses * jours) as f64,
                &format!("{NOTE_BANDE} · {}", texte_soin(motif.as_deref(), doses, jours, delai, note.as_deref())),
                Some(&code),
            ).await?;
            format!("bande {code}")
        }
        _ => return Err(AppError::Invalid("Choisissez qui est soigné : truie, bande, verrat ou porc".into())),
    };
    tx.commit().await?;
    db::journal(pool, &session.nom, "ajouter", "soin", &cible, "/soins").await;
    Ok(Redirect::to(&retour(&form, "/soins?ok=1")).into_response())
}

/// « Fait » sur une ligne de la partie À faire.
pub(super) async fn fait(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let acte = form_i64(&form, "acte_id").ok_or_else(|| AppError::Invalid("Acte manquant".into()))?;
    let bande = form_i64(&form, "bande_id").unwrap_or(0);
    let date = form_date_or_today(&form, "date")?;
    let doses = match form_text(&form, "doses") {
        None => 0.0,
        Some(_) => form_f64(&form, "doses")
            .filter(|d| *d >= 0.0)
            .ok_or_else(|| AppError::Invalid("Doses utilisées : un nombre".into()))?,
    };
    let acte_info: (String, Option<String>) = sqlx::query_as("SELECT libelle,produit FROM acteprotocole WHERE id=?")
        .bind(acte).fetch_optional(&state.pool).await?.ok_or(AppError::NotFound)?;
    let code: Option<String> = sqlx::query_scalar("SELECT code FROM bande WHERE id=?")
        .bind(bande).fetch_optional(&state.pool).await?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    if form.get("qui").map(String::as_str) == Some("portees") {
        sqlx::query("UPDATE soinportee SET date_realisee=? WHERE protocole_id=? AND COALESCE(bande_id,0)=? AND date_realisee IS NULL AND date(date_prevue)<=date(?,'+7 day')")
            .bind(&date).bind(acte).bind(bande).bind(&date).execute(&mut *tx).await?;
    } else {
        if code.is_none() {
            return Err(AppError::Invalid("Bande manquante".into()));
        }
        let deja: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM acterealise WHERE acte_id=? AND bande_id=? AND date_realise=?")
            .bind(acte).bind(bande).bind(&date).fetch_one(&mut *tx).await?;
        if deja == 0 {
            sqlx::query("INSERT INTO acterealise(acte_id,bande_id,date_realise,note) VALUES(?,?,?,?)")
                .bind(acte).bind(bande).bind(&date).bind(format!("Page Soins ({})", session.identifiant))
                .execute(&mut *tx).await?;
        }
    }
    if doses > 0.0 {
        if let Some(produit) = acte_info.1.as_deref() {
            traitement_stock::sortie(
                &mut tx, produit, &date, doses,
                &format!("Plan sanitaire : {}", acte_info.0),
                code.as_deref(),
            ).await?;
        }
    }
    tx.commit().await?;
    Ok(Redirect::to("/soins?ok=fait").into_response())
}

/// Annule un soin de bande (remet les doses en stock).
pub(super) async fn annuler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let id = form_i64(&form, "id").ok_or_else(|| AppError::Invalid("Soin manquant".into()))?;
    let mut tx = state.pool.begin_with("BEGIN IMMEDIATE").await?;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM mouvementpharmacie WHERE id=? AND type='sortie' AND note LIKE 'Soin de bande%'")
        .bind(id).fetch_one(&mut *tx).await?;
    if n == 0 {
        return Err(AppError::NotFound);
    }
    traitement_stock::restituer(&mut tx, id).await?;
    tx.commit().await?;
    Ok(Redirect::to("/soins?ok=annule#historique").into_response())
}

/// Suppression d'un soin de porc : les doses sorties par la page Soins
/// reviennent en stock.
pub(super) async fn rendre_doses_porc(conn: &mut sqlx::SqliteConnection, id: i64) -> AppResult<()> {
    let mouvement: Option<i64> = sqlx::query_scalar("SELECT id FROM mouvementpharmacie WHERE type='sortie' AND note LIKE ? ORDER BY id DESC LIMIT 1")
        .bind(format!("Soin porc #{id} :%"))
        .fetch_optional(&mut *conn)
        .await?;
    if let Some(mouvement) = mouvement {
        traitement_stock::restituer(conn, mouvement).await?;
    }
    Ok(())
}

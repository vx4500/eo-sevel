//! Vente directe : disponibilité des morceaux avant et après inventaire,
//! estimation du nombre de cochons à tuer, renvoi du code de modification et
//! annonce de l'ouverture des commandes aux clients.
//!
//! Règle de disponibilité : à l'ouverture d'une vente, tous les morceaux sont
//! disponibles sans limite (`quantite_disponible` NULL). Les commandes reçues
//! servent à estimer combien de cochons tuer. Quand l'éleveur déclare son
//! inventaire (quantité réellement produite par morceau), le reste disponible
//! devient « produit − déjà commandé » et les commandes sont alors limitées.

use super::*;

/// Adresse publique valide (http ou https, sans espace), sans barre finale.
fn adresse_valide(value: &str) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))?;
    if rest.is_empty() || value.chars().any(char::is_whitespace) {
        return None;
    }
    Some(value.trim_end_matches('/').to_string())
}

/// Hôte vu dans la requête (derrière le tunnel Cloudflare : X-Forwarded-*).
fn hote_requete(headers: &HeaderMap) -> Option<(String, String)> {
    let get = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let host = get("x-forwarded-host").or_else(|| get("host"))?;
    if host.chars().any(|c| c.is_whitespace() || c == '/' || c == '@') {
        return None;
    }
    let proto = match get("x-forwarded-proto").as_deref() {
        Some("http") => "http",
        Some("https") => "https",
        _ if est_hote_local(&host) => "http",
        _ => "https",
    };
    Some((proto.to_string(), host))
}

fn est_hote_local(host: &str) -> bool {
    // Retire le port : « [::1]:8080 », « 192.168.1.10:8080 », « nom:8080 ».
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else if host.matches(':').count() == 1 {
        host.split(':').next().unwrap_or(host)
    } else {
        host
    };
    name.eq_ignore_ascii_case("localhost")
        || name.parse::<std::net::IpAddr>().is_ok()
        || !name.contains('.')
}

/// Adresse à mettre dans les liens envoyés aux clients.
///
/// 1. le paramètre « Adresse publique du logiciel » s'il est une vraie URL ;
/// 2. sinon l'adresse par laquelle arrive la requête. Pour un envoi groupé
///    (`exiger_publique`), une adresse locale (IP, localhost) est refusée :
///    le lien ne fonctionnerait pas chez les clients.
pub(super) async fn adresse_publique(
    pool: &SqlitePool,
    headers: Option<&HeaderMap>,
    exiger_publique: bool,
) -> AppResult<Option<String>> {
    let configured: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='public_url'")
            .fetch_optional(pool)
            .await?;
    if let Some(url) = configured.as_deref().and_then(adresse_valide) {
        return Ok(Some(url));
    }
    let Some((proto, host)) = headers.and_then(hote_requete) else {
        return Ok(None);
    };
    if exiger_publique && est_hote_local(&host) {
        return Ok(None);
    }
    Ok(Some(format!("{proto}://{host}")))
}

/// Le paramètre « Adresse publique du logiciel » est-il une URL utilisable ?
pub(super) async fn adresse_publique_configuree(pool: &SqlitePool) -> AppResult<bool> {
    let configured: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='public_url'")
            .fetch_optional(pool)
            .await?;
    Ok(configured.as_deref().and_then(adresse_valide).is_some())
}

pub(super) fn verifier_adresse_publique(value: &str) -> AppResult<()> {
    if value.trim().is_empty() || adresse_valide(value).is_some() {
        Ok(())
    } else {
        Err(AppError::Invalid(
            "L’adresse publique du logiciel doit être une adresse web commençant par https:// (par exemple https://elevage.mon-domaine.fr), pas l’adresse postale.".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Renvoi du code de modification (côté éleveur)
// ---------------------------------------------------------------------------

pub(super) async fn renvoyer_code(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let order: Option<(Option<i64>, String, Option<String>, Option<String>, Option<String>, f64)> =
        sqlx::query_as("SELECT client_id,nom_client,email,token_modification,code_modification,total FROM commandeventedirecte WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let Some((client_id, name, email, token, code, total)) = order else {
        return Err(AppError::NotFound);
    };
    let Some(email) = email.filter(|v| v.contains('@')) else {
        return Err(AppError::Invalid(
            "Cette commande n’a pas d’adresse e-mail : donnez le code au client par téléphone.".into(),
        ));
    };
    let (Some(token), Some(code)) = (token, code) else {
        return Err(AppError::Invalid(
            "Cette commande a été saisie sans code de modification.".into(),
        ));
    };
    let lines: Vec<(String, f64, String, f64)> = sqlx::query_as(
        "SELECT nom_produit,quantite,unite,total_ligne FROM lignecommandeventedirecte WHERE commande_id=? ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await?;
    let base = adresse_publique(&state.pool, Some(&headers), true).await?;
    let recap = parity::RecapCommande {
        order_id: id,
        client_id,
        destinataire: &email,
        nom_client: &name,
        token: &token,
        code: &code,
        total,
        lignes: &lines,
        adresse: base.as_deref(),
    };
    let result = parity::envoyer_recap_commande(&state.pool, recap).await;
    db::journal(
        &state.pool,
        &session.nom,
        "renvoyer_code",
        "commande_vente_directe",
        &format!("commande {id} : {}", if result.is_ok() { "envoyé" } else { "échec" }),
        "/vente-directe/commandes",
    )
    .await;
    let flash = match result {
        Ok(()) => format!("code=envoye&cmd={id}"),
        Err(AppError::Invalid(message)) => format!("cmd={id}&code_erreur={}", encode_query(&message)),
        Err(error) => return Err(error),
    };
    Ok(Redirect::to(&format!("/vente-directe/commandes?{flash}#cmd-{id}")).into_response())
}

// ---------------------------------------------------------------------------
// Disponibilité : ouverture sans limite, inventaire, estimation des cochons
// ---------------------------------------------------------------------------

/// À l'ouverture d'une vente : tous les morceaux redeviennent disponibles sans
/// limite et l'inventaire de la vente est « non déclaré ».
pub(super) async fn ouvrir_sans_limite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    session_id: i64,
) -> AppResult<()> {
    sqlx::query("UPDATE produitventedirecte SET quantite_disponible=NULL")
        .execute(&mut **tx)
        .await?;
    sqlx::query("UPDATE sessionventedirecte SET inventaire_declare_le=NULL WHERE id=?")
        .bind(session_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Nombre de cochons nécessaires pour une quantité commandée, d'après ce
/// qu'un cochon donne de ce morceau. `None` si le rendement n'est pas connu.
pub(super) fn cochons_pour(commande: f64, par_porc: Option<f64>) -> Option<f64> {
    let par_porc = par_porc.filter(|v| *v > 0.0)?;
    Some(commande.max(0.0) / par_porc)
}

/// Arrondi au cochon supérieur, sans compter les erreurs d'arrondi (2,0000001).
pub(super) fn arrondi_cochons(value: f64) -> i64 {
    if value <= 0.0 {
        0
    } else {
        (value - 1e-6).ceil().max(1.0) as i64
    }
}

/// Lignes « produit / commandé / rendement / cochons » de la vente active.
pub(super) async fn estimation_vente(pool: &SqlitePool, session_id: i64) -> AppResult<Value> {
    let rows: Vec<(i64, String, String, bool, Option<f64>, Option<f64>, f64)> = sqlx::query_as(
        "SELECT p.id,p.nom,p.unite,p.actif,p.quantite_disponible,p.quantite_par_porc,
                CAST(COALESCE((SELECT SUM(l.quantite) FROM lignecommandeventedirecte l JOIN commandeventedirecte c ON c.id=l.commande_id
                  WHERE l.produit_id=p.id AND c.session_vente_id=? AND c.statut<>'annulee'),0) AS REAL)
         FROM produitventedirecte p ORDER BY p.ordre,p.nom",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await?;
    let kg_par_porc: Option<f64> =
        sqlx::query_scalar("SELECT kg_par_porc FROM reglageventedirecte WHERE id=1")
            .fetch_optional(pool)
            .await?
            .flatten();
    let mut lignes = Vec::new();
    let mut max_cochons: f64 = 0.0;
    let mut produit_limitant: Option<String> = None;
    let mut total_kg = 0.0;
    let mut sans_rendement = 0;
    for (id, nom, unite, actif, dispo, par_porc, commande) in rows {
        if unite == "kg" {
            total_kg += commande;
        }
        let cochons = cochons_pour(commande, par_porc);
        if let Some(value) = cochons {
            if value > max_cochons {
                max_cochons = value;
                produit_limitant = Some(nom.clone());
            }
        } else if commande > 0.0 {
            sans_rendement += 1;
        }
        lignes.push(json!({
            "id": id, "nom": nom, "unite": unite, "actif": actif,
            "quantite_disponible": dispo, "quantite_par_porc": par_porc,
            "commande": (commande * 100.0).round() / 100.0,
            "cochons": cochons.map(|v| (v * 10.0).round() / 10.0),
        }));
    }
    let par_poids = cochons_pour(total_kg, kg_par_porc);
    let estimation = max_cochons.max(par_poids.unwrap_or(0.0));
    Ok(json!({
        "lignes": lignes,
        "total_kg": (total_kg * 100.0).round() / 100.0,
        "kg_par_porc": kg_par_porc,
        "cochons_par_poids": par_poids.map(|v| (v * 10.0).round() / 10.0),
        "cochons_par_morceau": (max_cochons > 0.0).then(|| (max_cochons * 10.0).round() / 10.0),
        "produit_limitant": produit_limitant,
        "cochons_a_tuer": arrondi_cochons(estimation),
        "sans_rendement": sans_rendement,
    }))
}

async fn vente_active(pool: &SqlitePool) -> AppResult<Option<i64>> {
    Ok(sqlx::query_scalar("SELECT id FROM sessionventedirecte WHERE active=1 ORDER BY id DESC LIMIT 1")
        .fetch_optional(pool)
        .await?)
}

/// Enregistre ce qu'un cochon donne de chaque morceau (et le poids vendable
/// moyen par cochon), pour l'estimation.
pub(super) async fn rendements(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM produitventedirecte")
        .fetch_all(&state.pool)
        .await?;
    let mut tx = state.pool.begin().await?;
    for id in ids {
        let key = format!("par_porc_{id}");
        if !form.contains_key(&key) {
            continue;
        }
        let value = form_f64(&form, &key).filter(|v| *v > 0.0 && *v < 10_000.0);
        sqlx::query("UPDATE produitventedirecte SET quantite_par_porc=? WHERE id=?")
            .bind(value)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if form.contains_key("kg_par_porc") {
        let value = form_f64(&form, "kg_par_porc").filter(|v| *v > 0.0 && *v < 1_000.0);
        sqlx::query("INSERT INTO reglageventedirecte(id,kg_par_porc) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET kg_par_porc=excluded.kg_par_porc")
            .bind(value)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Redirect::to("/vente-directe#cette-vente").into_response())
}

/// Déclare l'inventaire de la vente active : pour chaque morceau, la quantité
/// réellement produite. Le reste disponible = produit − déjà commandé.
/// Un champ vide laisse le morceau sans limite.
pub(super) async fn inventaire_declarer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let Some(session_id) = vente_active(&state.pool).await? else {
        return Err(AppError::Invalid(
            "Aucune vente active : créez ou activez d’abord une vente.".into(),
        ));
    };
    let mut tx = state.pool.begin().await?;
    let rows: Vec<(i64, String, f64)> = sqlx::query_as(
        "SELECT p.id,p.nom,CAST(COALESCE((SELECT SUM(l.quantite) FROM lignecommandeventedirecte l JOIN commandeventedirecte c ON c.id=l.commande_id
            WHERE l.produit_id=p.id AND c.session_vente_id=? AND c.statut<>'annulee'),0) AS REAL) FROM produitventedirecte p",
    )
    .bind(session_id)
    .fetch_all(&mut *tx)
    .await?;
    let mut resume = Vec::new();
    for (id, nom, commande) in rows {
        let key = format!("inventaire_{id}");
        let Some(raw) = form.get(&key) else { continue };
        let stock = if raw.trim().is_empty() {
            None
        } else {
            let produit = form_f64(&form, &key)
                .filter(|v| *v >= 0.0 && *v < 100_000.0)
                .ok_or_else(|| AppError::Invalid(format!("Quantité invalide pour {nom}")))?;
            Some(((produit - commande).max(0.0) * 1000.0).round() / 1000.0)
        };
        sqlx::query("UPDATE produitventedirecte SET quantite_disponible=? WHERE id=?")
            .bind(stock)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        resume.push(format!(
            "{nom}: {}",
            stock.map(|v| format!("reste {v}")).unwrap_or_else(|| "sans limite".into())
        ));
    }
    sqlx::query("UPDATE sessionventedirecte SET inventaire_declare_le=CURRENT_TIMESTAMP WHERE id=?")
        .bind(session_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    db::journal(
        &state.pool,
        &session.nom,
        "inventaire",
        "vente_directe",
        &resume.join(" ; "),
        "/vente-directe",
    )
    .await;
    Ok(Redirect::to("/vente-directe?inventaire=ok#cette-vente").into_response())
}

/// Revient à « tout disponible » (annule l'inventaire déclaré).
pub(super) async fn tout_disponible(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin().await?;
    if let Some(session_id) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM sessionventedirecte WHERE active=1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&mut *tx)
    .await?
    {
        ouvrir_sans_limite(&mut tx, session_id).await?;
    } else {
        sqlx::query("UPDATE produitventedirecte SET quantite_disponible=NULL")
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    db::journal(
        &state.pool,
        &session.nom,
        "inventaire",
        "vente_directe",
        "tous les morceaux disponibles sans limite",
        "/vente-directe",
    )
    .await;
    Ok(Redirect::to("/vente-directe#cette-vente").into_response())
}

// ---------------------------------------------------------------------------
// Annonce de l'ouverture des commandes
// ---------------------------------------------------------------------------

/// Clients à qui l'annonce peut partir. `anciens` ajoute les clients ayant
/// déjà commandé, sans accord enregistré mais jamais désinscrits.
async fn destinataires_annonce(
    pool: &SqlitePool,
    anciens: bool,
) -> AppResult<Vec<(i64, String, String, bool)>> {
    let rows: Vec<(i64, String, String, bool)> = sqlx::query_as(
        "SELECT id,nom,trim(email),newsletter_email=1 FROM clientventedirecte k
         WHERE trim(COALESCE(email,'')) LIKE '%_@_%' AND desinscrit_le IS NULL
           AND (newsletter_email=1 OR (? AND EXISTS(SELECT 1 FROM commandeventedirecte c WHERE c.client_id=k.id)))
         ORDER BY nom COLLATE NOCASE,id",
    )
    .bind(anciens)
    .fetch_all(pool)
    .await?;
    // Un même e-mail ne reçoit l'annonce qu'une fois.
    let mut seen = HashSet::new();
    Ok(rows
        .into_iter()
        .filter(|(_, _, email, _)| seen.insert(email.to_lowercase()))
        .collect())
}

fn texte_annonce_defaut(nom_vente: &str, limite: Option<&str>, livraison: Option<&str>) -> String {
    let fr = |d: &str| {
        NaiveDate::parse_from_str(d, "%Y-%m-%d")
            .map(|d| d.format("%d/%m/%Y").to_string())
            .unwrap_or_else(|_| d.to_string())
    };
    let mut text = format!(
        "Bonjour,\n\nLes commandes de viande de porc sont ouvertes pour « {nom_vente} »."
    );
    if let Some(d) = limite {
        text.push_str(&format!("\nVous pouvez commander jusqu’au {}.", fr(d)));
    }
    if let Some(d) = livraison {
        text.push_str(&format!("\nLivraison prévue le {}.", fr(d)));
    }
    text.push_str("\n\nÀ bientôt !");
    text
}

pub(super) fn contenu_annonce(message: &str, lien_commande: &str, lien_desinscription: Option<&str>) -> String {
    let corps = parity::escape_html(message).replace('\n', "<br>");
    let mut html = format!(
        "<div style=\"font-family:Arial,sans-serif;font-size:16px;line-height:1.5\"><p>{corps}</p><p style=\"margin:24px 0\"><a href=\"{lien}\" style=\"background:#1a7f37;color:#fff;padding:12px 20px;border-radius:8px;text-decoration:none;font-weight:bold\">Passer ma commande</a></p><p style=\"font-size:13px;color:#555\">Lien direct : {lien}</p>",
        lien = parity::escape_html(lien_commande)
    );
    if let Some(lien) = lien_desinscription {
        html.push_str(&format!(
            "<p style=\"font-size:12px;color:#777\">Vous recevez ce message parce que vous avez déjà commandé chez nous. <a href=\"{}\">Ne plus recevoir ces messages</a>.</p>",
            parity::escape_html(lien)
        ));
    }
    html.push_str("</div>");
    html
}

pub(super) async fn annonce_page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    let active: Option<(i64, String, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id,nom,date_limite_commandes,date_livraison,annonce_envoyee_le FROM sessionventedirecte WHERE active=1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_optional(&state.pool)
    .await?;
    let enregistre: Option<(Option<String>, Option<String>)> = match &active {
        Some((id, ..)) => sqlx::query_as("SELECT annonce_sujet,annonce_message FROM sessionventedirecte WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?,
        None => None,
    };
    let (sujet_enregistre, message_enregistre) = enregistre.unwrap_or((None, None));
    let texte_enregistre = sujet_enregistre.is_some() || message_enregistre.is_some();
    let ouvertes: i64 = sqlx::query_scalar("SELECT COALESCE((SELECT commandes_ouvertes FROM reglageventedirecte WHERE id=1),1)")
        .fetch_one(&state.pool)
        .await?;
    let anciens = query.get("anciens").map(String::as_str) == Some("1");
    let destinataires = destinataires_annonce(&state.pool, anciens).await?;
    let avec_accord = destinataires.iter().filter(|d| d.3).count();
    let sans_accord: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM clientventedirecte k WHERE trim(COALESCE(email,'')) LIKE '%_@_%' AND desinscrit_le IS NULL AND newsletter_email=0 AND EXISTS(SELECT 1 FROM commandeventedirecte c WHERE c.client_id=k.id)",
    )
    .fetch_one(&state.pool)
    .await?;
    let base = adresse_publique(&state.pool, Some(&headers), true).await?;
    let brevo_pret = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM reglagecommunicationventedirecte WHERE id=1 AND trim(COALESCE(brevo_api_key,''))<>'' AND sender_email LIKE '%@%'",
    )
    .fetch_one(&state.pool)
    .await?
        > 0;
    let expediteur: Option<String> =
        sqlx::query_scalar("SELECT sender_email FROM reglagecommunicationventedirecte WHERE id=1")
            .fetch_optional(&state.pool)
            .await?
            .flatten();
    let (sujet_defaut, message_defaut) = match &active {
        Some((_, nom, limite, livraison, _)) => (
            format!("Les commandes sont ouvertes : {nom}"),
            texte_annonce_defaut(nom, limite.as_deref(), livraison.as_deref()),
        ),
        None => (String::new(), String::new()),
    };
    let sujet = sujet_enregistre.filter(|v| !v.trim().is_empty()).unwrap_or(sujet_defaut);
    let message = message_enregistre.filter(|v| !v.trim().is_empty()).unwrap_or(message_defaut);
    let lien = base.as_ref().map(|b| format!("{b}/commande"));
    let apercu = lien
        .as_deref()
        .map(|l| contenu_annonce(&message, l, Some(&format!("{}/desinscription/…", base.as_deref().unwrap_or("")))));
    let mut ctx = context(&session);
    ctx.insert(
        "vente".into(),
        active
            .as_ref()
            .map(|(id, nom, limite, livraison, envoyee)| json!({"id":id,"nom":nom,"date_limite_commandes":limite,"date_livraison":livraison,"annonce_envoyee_le":envoyee}))
            .unwrap_or(Value::Null),
    );
    ctx.insert("commandes_ouvertes".into(), json!(ouvertes == 1));
    ctx.insert(
        "destinataires".into(),
        json!(destinataires.iter().map(|(id, nom, email, accord)| json!({"id":id,"nom":nom,"email":email,"accord":accord})).collect::<Vec<_>>()),
    );
    ctx.insert("avec_accord".into(), json!(avec_accord));
    ctx.insert("sans_accord".into(), json!(sans_accord));
    ctx.insert("anciens".into(), json!(anciens));
    ctx.insert("adresse".into(), json!(base));
    ctx.insert("lien_commande".into(), json!(lien));
    ctx.insert("brevo_pret".into(), json!(brevo_pret));
    ctx.insert("expediteur".into(), json!(expediteur));
    ctx.insert("sujet".into(), json!(sujet));
    ctx.insert("message".into(), json!(message));
    ctx.insert("apercu".into(), json!(apercu));
    ctx.insert("envoyes".into(), json!(query.get("envoyes")));
    ctx.insert("echecs".into(), json!(query.get("echecs")));
    ctx.insert("essai".into(), json!(query.get("essai")));
    ctx.insert("texte_enregistre".into(), json!(texte_enregistre));
    ctx.insert("flash_annonce".into(), json!(query.get("fait")));
    ctx.insert("ajoutees".into(), json!(query.get("ajoutees")));
    render(&state, "vente_directe_annonce.html", Value::Object(ctx))
}

struct Annonce {
    sujet: String,
    message: String,
    lien_commande: String,
    base: String,
    session_id: i64,
}

async fn preparer_annonce(
    state: &AppState,
    session: &SessionData,
    form: &HashMap<String, String>,
    headers: &HeaderMap,
) -> AppResult<Annonce> {
    require_writer(session)?;
    verify_csrf(session, form)?;
    let Some(session_id) = vente_active(&state.pool).await? else {
        return Err(AppError::Invalid("Aucune vente active.".into()));
    };
    let sujet = form_text(form, "sujet")
        .filter(|v| v.chars().count() <= 200)
        .ok_or_else(|| AppError::Invalid("Sujet obligatoire (200 caractères maximum).".into()))?;
    let message = form_text(form, "message")
        .filter(|v| v.chars().count() <= 5000)
        .ok_or_else(|| AppError::Invalid("Message obligatoire.".into()))?;
    enregistrer_texte(&state.pool, session_id, &sujet, &message).await?;
    let base = adresse_publique(&state.pool, Some(headers), true)
        .await?
        .ok_or_else(|| AppError::Invalid("Adresse publique du logiciel inconnue : renseignez-la dans Paramètres (https://…) pour que le lien de commande fonctionne chez les clients.".into()))?;
    Ok(Annonce {
        lien_commande: format!("{base}/commande"),
        base,
        sujet,
        message,
        session_id,
    })
}

async fn enregistrer_texte(pool: &SqlitePool, session_id: i64, sujet: &str, message: &str) -> AppResult<()> {
    sqlx::query("UPDATE sessionventedirecte SET annonce_sujet=?,annonce_message=? WHERE id=?")
        .bind(sujet)
        .bind(message)
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}

fn retour_annonce(form: &HashMap<String, String>, suite: &str) -> Response {
    let anciens = if form.get("anciens").map(String::as_str) == Some("1") { "anciens=1&" } else { "" };
    Redirect::to(&format!("/vente-directe/annonce?{anciens}{suite}")).into_response()
}

/// « Enregistrer le texte » : garde le sujet et le message modifiés pour la
/// vente active, sans rien envoyer. Vide = retour au texte proposé.
pub(super) async fn annonce_enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let Some(session_id) = vente_active(&state.pool).await? else {
        return Err(AppError::Invalid("Aucune vente active.".into()));
    };
    if form.get("reinitialiser").map(String::as_str) == Some("1") {
        sqlx::query("UPDATE sessionventedirecte SET annonce_sujet=NULL,annonce_message=NULL WHERE id=?")
            .bind(session_id)
            .execute(&state.pool)
            .await?;
        return Ok(retour_annonce(&form, "fait=reinitialise"));
    }
    let sujet = form_text(&form, "sujet").unwrap_or_default();
    let message = form_text(&form, "message").unwrap_or_default();
    if sujet.chars().count() > 200 || message.chars().count() > 5000 {
        return Err(AppError::Invalid("Texte trop long (sujet 200 caractères, message 5000).".into()));
    }
    enregistrer_texte(&state.pool, session_id, &sujet, &message).await?;
    Ok(retour_annonce(&form, "fait=enregistre"))
}

/// Adresses saisies à la main : une par ligne (ou séparées par des virgules,
/// points-virgules, espaces), avec un nom facultatif « Nom <adresse> ».
fn lire_adresses(texte: &str) -> (Vec<(String, String)>, Vec<String>) {
    let mut bonnes: Vec<(String, String)> = Vec::new();
    let mut mauvaises = Vec::new();
    for ligne in texte.split(['\n', ';', ',']) {
        let ligne = ligne.trim();
        if ligne.is_empty() {
            continue;
        }
        let (nom, reste) = match (ligne.find('<'), ligne.rfind('>')) {
            (Some(a), Some(b)) if a < b => (ligne[..a].trim().trim_matches('"').trim().to_string(), ligne[a + 1..b].to_string()),
            _ => (String::new(), ligne.to_string()),
        };
        let mots: Vec<&str> = reste.split_whitespace().collect();
        let (nom, emails): (String, Vec<&str>) = if nom.is_empty() {
            let emails: Vec<&str> = mots.iter().copied().filter(|m| m.contains('@')).collect();
            let nom = mots.iter().copied().filter(|m| !m.contains('@')).collect::<Vec<_>>().join(" ");
            (nom, emails)
        } else {
            (nom, mots)
        };
        if emails.is_empty() {
            mauvaises.push(ligne.to_string());
        }
        for email in emails {
            let email = email.trim_matches(|c: char| matches!(c, '<' | '>' | '"' | '\'' | '(' | ')')).to_lowercase();
            let valide = email.len() <= 200
                && email.split_once('@').is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.') && !b.ends_with('.') && !b.contains('@'));
            if !valide {
                mauvaises.push(email);
            } else if !bonnes.iter().any(|(_, e)| *e == email) {
                bonnes.push((nom.clone(), email));
            }
        }
    }
    (bonnes, mauvaises)
}

/// Ajoute des destinataires à la main : client existant → son accord est
/// enregistré ; sinon une fiche client est créée (sans commande).
pub(super) async fn annonce_ajouter_adresses(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let (adresses, mauvaises) = lire_adresses(form.get("adresses").map(String::as_str).unwrap_or(""));
    if !mauvaises.is_empty() {
        return Err(AppError::Invalid(format!(
            "Adresse(s) non reconnue(s), rien n’a été ajouté : {}. Corrigez et réessayez.",
            mauvaises.join(", ")
        )));
    }
    if adresses.is_empty() {
        return Err(AppError::Invalid("Saisissez au moins une adresse e-mail.".into()));
    }
    let mut tx = state.pool.begin().await?;
    for (nom, email) in &adresses {
        let existants: Vec<i64> = sqlx::query_scalar("SELECT id FROM clientventedirecte WHERE lower(trim(email))=?")
            .bind(email)
            .fetch_all(&mut *tx)
            .await?;
        if existants.is_empty() {
            let nom = if nom.is_empty() { email.split('@').next().unwrap_or(email).to_string() } else { nom.clone() };
            sqlx::query("INSERT INTO clientventedirecte(nom,email,telephone,newsletter_email,newsletter_sms,cree_le,token_desinscription) VALUES(?,?,NULL,1,0,CURRENT_TIMESTAMP,?)")
                .bind(nom)
                .bind(email)
                .bind(uuid::Uuid::new_v4().simple().to_string())
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("UPDATE clientventedirecte SET newsletter_email=1,desinscrit_le=NULL WHERE lower(trim(email))=?")
                .bind(email)
                .execute(&mut *tx)
                .await?;
        }
    }
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "ajouter", "annonce_vente_directe", &format!("{} adresse(s)", adresses.len()), "/vente-directe/annonce").await;
    Ok(retour_annonce(&form, &format!("ajoutees={}", adresses.len())))
}

/// Retire une adresse de la liste d'annonce (le client reste dans le fichier).
pub(super) async fn annonce_retirer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("UPDATE clientventedirecte SET newsletter_email=0,desinscrit_le=COALESCE(desinscrit_le,CURRENT_TIMESTAMP) WHERE id=?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(retour_annonce(&form, "fait=retire"))
}

/// Essai : un seul e-mail, à l'adresse expéditrice (celle de l'éleveur).
pub(super) async fn annonce_essai(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    let annonce = preparer_annonce(&state, &session, &form, &headers).await?;
    let settings = parity::comm_settings(&state.pool).await?;
    let contenu = contenu_annonce(
        &annonce.message,
        &annonce.lien_commande,
        Some(&format!("{}/desinscription/essai", annonce.base)),
    );
    let to = settings.sender_email.clone();
    let result = parity::brevo_send(&settings, "email", &to, "Essai", &format!("[Essai] {}", annonce.sujet), &contenu).await;
    parity::log_message(&state.pool, None, "email", "annonce_essai", &to, &contenu, &result).await?;
    if let Err(error) = result {
        return Err(AppError::Invalid(format!("Échec de l’envoi d’essai : {error}")));
    }
    Ok(Redirect::to("/vente-directe/annonce?essai=ok").into_response())
}

/// Envoi réel de l'annonce. Exige la case de confirmation et le nombre de
/// destinataires vu à l'écran (si la liste a changé entre-temps, rien ne part).
pub(super) async fn annonce_envoyer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    headers: HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    let annonce = preparer_annonce(&state, &session, &form, &headers).await?;
    if form.get("confirmer").map(String::as_str) != Some("1") {
        return Err(AppError::Invalid(
            "Cochez la case de confirmation avant l’envoi.".into(),
        ));
    }
    let anciens = form.get("anciens").map(String::as_str) == Some("1");
    let destinataires = destinataires_annonce(&state.pool, anciens).await?;
    if form_i64(&form, "nb_attendu") != Some(destinataires.len() as i64) {
        return Err(AppError::Invalid(
            "La liste des destinataires a changé : rechargez la page et vérifiez avant d’envoyer.".into(),
        ));
    }
    if destinataires.is_empty() {
        return Err(AppError::Invalid("Aucun destinataire.".into()));
    }
    let settings = parity::comm_settings(&state.pool).await?;
    let (mut envoyes, mut echecs) = (0, 0);
    for (client_id, nom, email, _) in destinataires {
        let token: String = sqlx::query_scalar("SELECT token_desinscription FROM clientventedirecte WHERE id=?")
            .bind(client_id)
            .fetch_one(&state.pool)
            .await?;
        let token = if token.trim().is_empty() {
            let token = uuid::Uuid::new_v4().simple().to_string();
            sqlx::query("UPDATE clientventedirecte SET token_desinscription=? WHERE id=?")
                .bind(&token)
                .bind(client_id)
                .execute(&state.pool)
                .await?;
            token
        } else {
            token
        };
        let contenu = contenu_annonce(
            &annonce.message,
            &annonce.lien_commande,
            Some(&format!("{}/desinscription/{token}", annonce.base)),
        );
        let result = parity::brevo_send(&settings, "email", &email, &nom, &annonce.sujet, &contenu).await;
        parity::log_message(&state.pool, Some(client_id), "email", "annonce_ouverture", &email, &contenu, &result).await?;
        if result.is_ok() {
            envoyes += 1;
        } else {
            echecs += 1;
        }
    }
    sqlx::query("UPDATE sessionventedirecte SET annonce_envoyee_le=CURRENT_TIMESTAMP WHERE id=?")
        .bind(annonce.session_id)
        .execute(&state.pool)
        .await?;
    db::journal(
        &state.pool,
        &session.nom,
        "annonce",
        "vente_directe",
        &format!("{envoyes} envoyé(s), {echecs} échec(s)"),
        "/vente-directe/annonce",
    )
    .await;
    Ok(Redirect::to(&format!("/vente-directe/annonce?envoyes={envoyes}&echecs={echecs}")).into_response())
}

#[cfg(test)]
mod tests {
    use super::super::documents::tests::{session, state};
    use super::*;

    fn form(fields: &[(&str, &str)]) -> HashMap<String, String> {
        let mut result = HashMap::from([("csrf_token".into(), "test".into())]);
        result.extend(fields.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        result
    }

    async fn vente() -> AppState {
        let state = state().await;
        sqlx::raw_sql(
            "ALTER TABLE produitventedirecte ADD COLUMN quantite_par_porc REAL;
             ALTER TABLE sessionventedirecte ADD COLUMN inventaire_declare_le TEXT;
             ALTER TABLE sessionventedirecte ADD COLUMN annonce_envoyee_le TEXT;
             ALTER TABLE sessionventedirecte ADD COLUMN annonce_sujet TEXT;
             ALTER TABLE sessionventedirecte ADD COLUMN annonce_message TEXT;
             ALTER TABLE reglageventedirecte ADD COLUMN kg_par_porc REAL;
             ALTER TABLE clientventedirecte ADD COLUMN desinscrit_le TEXT;
             INSERT INTO produitventedirecte(id,nom,prix,unite,actif,ordre,quantite_disponible) VALUES
               (1,'Côte échine',12,'kg',1,1,0),(2,'Filet mignon',25,'kg',1,2,0),(3,'Tête',5,'pièce',1,3,2);
             INSERT INTO reglagecommunicationventedirecte(id,brevo_api_key,sender_email,sender_name) VALUES(1,'cle-de-test','ferme@example.fr','Ferme');
             INSERT INTO parametre(cle,valeur) VALUES('public_url','302 LD BASSE CHEVRIE');",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        state
    }

    fn headers_publics() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("host", HeaderValue::from_static("elevage.example.fr"));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        headers
    }

    async fn ouvrir_vente(state: &AppState) {
        vente_session_creer(
            State(state.clone()),
            Extension(session()),
            Form(form(&[("nom", "Vente décembre"), ("date_limite_commandes", "2099-12-17"), ("date_livraison", "2099-12-20")])),
        )
        .await
        .unwrap();
    }

    async fn commander(state: &AppState, fields: &[(&str, &str)]) -> Response {
        let mut f = HashMap::new();
        f.extend(fields.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        commande_post(State(state.clone()), headers_publics(), Form(f)).await.unwrap()
    }

    fn location(response: &Response) -> String {
        response.headers().get("location").unwrap().to_str().unwrap().to_string()
    }

    #[test]
    fn adresse_publique_refuse_l_adresse_postale() {
        assert_eq!(adresse_valide("302 LD BASSE CHEVRIE"), None);
        assert_eq!(adresse_valide("https://elevage.example.fr/"), Some("https://elevage.example.fr".into()));
        assert!(verifier_adresse_publique("302 LD BASSE CHEVRIE").is_err());
        assert!(verifier_adresse_publique("").is_ok());
        assert!(est_hote_local("192.168.1.10:8080"));
        assert!(est_hote_local("localhost:8080"));
        assert!(est_hote_local("[::1]:8080"));
        assert!(!est_hote_local("elevage.example.fr"));
    }

    #[test]
    fn estimation_cochons_arrondie_au_superieur() {
        assert_eq!(cochons_pour(30.0, Some(15.0)), Some(2.0));
        assert_eq!(cochons_pour(3.0, None), None);
        assert_eq!(arrondi_cochons(2.0000001), 2);
        assert_eq!(arrondi_cochons(2.1), 3);
        assert_eq!(arrondi_cochons(0.2), 1);
        assert_eq!(arrondi_cochons(0.0), 0);
    }

    #[tokio::test]
    async fn ouverture_tout_disponible_puis_inventaire_limite() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        // À l'ouverture : plus aucune limite, même pour les produits à 0.
        let limites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM produitventedirecte WHERE quantite_disponible IS NOT NULL")
            .fetch_one(&state.pool).await.unwrap();
        assert_eq!(limites, 0);
        let page = commande_page(State(state.clone()), Query(HashMap::new())).await.unwrap().0;
        assert!(page.contains("Filet mignon"), "un produit à 0 doit être proposé avant l’inventaire");

        // Grosse commande acceptée sans limite.
        let r = commander(&state, &[("nom_client", "Alice"), ("telephone", "0600000000"), ("email", "alice@example.fr"), ("q_1", "40"), ("q_2", "3")]).await;
        assert!(location(&r).starts_with("/commande/confirmation/"), "{}", location(&r));

        // Estimation : 40 kg d'échine à 10 kg/cochon = 4 ; 3 kg de filet à 1 kg = 3.
        rendements(State(state.clone()), Extension(session()), Form(form(&[("par_porc_1", "10"), ("par_porc_2", "1"), ("kg_par_porc", "60")]))).await.unwrap();
        let sid: i64 = sqlx::query_scalar("SELECT id FROM sessionventedirecte WHERE active=1").fetch_one(&state.pool).await.unwrap();
        let est = estimation_vente(&state.pool, sid).await.unwrap();
        assert_eq!(est["cochons_a_tuer"], json!(4));
        assert_eq!(est["produit_limitant"], json!("Côte échine"));
        assert_eq!(est["total_kg"], json!(43.0));

        // Inventaire : 45 kg d'échine produits → reste 5 ; filet 3 → reste 0 ; tête vide → sans limite.
        inventaire_declarer(State(state.clone()), Extension(session()), Form(form(&[("inventaire_1", "45"), ("inventaire_2", "3"), ("inventaire_3", "")]))).await.unwrap();
        let stocks: Vec<(i64, Option<f64>)> = sqlx::query_as("SELECT id,quantite_disponible FROM produitventedirecte ORDER BY id").fetch_all(&state.pool).await.unwrap();
        assert_eq!(stocks, vec![(1, Some(5.0)), (2, Some(0.0)), (3, None)]);
        let declare: Option<String> = sqlx::query_scalar("SELECT inventaire_declare_le FROM sessionventedirecte WHERE id=?").bind(sid).fetch_one(&state.pool).await.unwrap();
        assert!(declare.is_some());

        // Après l'inventaire : la limite s'applique.
        let r = commander(&state, &[("nom_client", "Bob"), ("telephone", "0611111111"), ("q_1", "6")]).await;
        assert_eq!(location(&r), "/commande?err=stock-insuffisant");
        let r = commander(&state, &[("nom_client", "Bob"), ("telephone", "0611111111"), ("q_1", "5")]).await;
        assert!(location(&r).starts_with("/commande/confirmation/"));
        let page = commande_page(State(state.clone()), Query(HashMap::new())).await.unwrap().0;
        assert!(!page.contains("Filet mignon"), "un produit épuisé après inventaire n’est plus proposé");

        // Retour à « tout disponible ».
        tout_disponible(State(state.clone()), Extension(session()), Form(form(&[]))).await.unwrap();
        let limites: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM produitventedirecte WHERE quantite_disponible IS NOT NULL")
            .fetch_one(&state.pool).await.unwrap();
        assert_eq!(limites, 0);
    }

    #[tokio::test]
    async fn code_de_modification_envoye_avec_un_lien_qui_marche() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        parity::vider_envois_de_test();
        let r = commander(&state, &[("nom_client", "Alice <b>"), ("telephone", "0600000000"), ("email", "alice@example.fr"), ("q_1", "2")]).await;
        let token = location(&r).trim_start_matches("/commande/confirmation/").to_string();
        let (code, envoye): (String, Option<String>) = sqlx::query_as("SELECT code_modification,recap_envoye_le FROM commandeventedirecte WHERE token_modification=?")
            .bind(&token).fetch_one(&state.pool).await.unwrap();
        assert!(envoye.is_some(), "le récapitulatif doit être marqué envoyé");
        let envois = parity::envois_de_test();
        let mail = envois.iter().find(|e| e.0 == "alice@example.fr").expect("e-mail au client");
        assert!(mail.2.contains(&code), "le code doit être dans l’e-mail");
        // L'adresse postale du paramètre est ignorée : le lien vient de l'hôte public.
        assert!(mail.2.contains(&format!("https://elevage.example.fr/commande/modifier/{token}")), "{}", mail.2);
        assert!(!mail.2.contains("302 LD BASSE CHEVRIE"));
        assert!(!mail.2.contains("<b>"), "le nom du client est échappé");
        assert!(!mail.2.contains("cle-de-test"), "jamais de clé API dans le contenu");

        // La page de confirmation dit au client que l'e-mail est parti.
        let page = commande_confirmation(State(state.clone()), Path(token.clone())).await.unwrap().0;
        assert!(page.contains(&code));
        assert!(page.contains("alice@example.fr"));

        // Le bon code ouvre la modification, un mauvais non.
        let r = commande_client_modifier(State(state.clone()), Path(token.clone()), headers_publics(), Form(HashMap::from([("code_modification".into(), "FAUX".into())]))).await.unwrap();
        let body = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("Code incorrect"));

        // Renvoi par l'éleveur : nouvel e-mail avec le même code.
        let id: i64 = sqlx::query_scalar("SELECT id FROM commandeventedirecte WHERE token_modification=?").bind(&token).fetch_one(&state.pool).await.unwrap();
        parity::vider_envois_de_test();
        let r = renvoyer_code(State(state.clone()), Extension(session()), Path(id), headers_publics(), Form(form(&[]))).await.unwrap();
        assert!(location(&r).contains("code=envoye"));
        let envois = parity::envois_de_test();
        assert_eq!(envois.len(), 1);
        assert!(envois[0].2.contains(&code));
    }

    #[tokio::test]
    async fn renvoi_du_code_refuse_un_lien_local() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        let r = commander(&state, &[("nom_client", "Alice"), ("telephone", "06"), ("email", "alice@example.fr"), ("q_1", "1")]).await;
        let token = location(&r).trim_start_matches("/commande/confirmation/").to_string();
        let id: i64 = sqlx::query_scalar("SELECT id FROM commandeventedirecte WHERE token_modification=?").bind(&token).fetch_one(&state.pool).await.unwrap();
        parity::vider_envois_de_test();
        let mut local = HeaderMap::new();
        local.insert("host", HeaderValue::from_static("192.168.1.10:8080"));
        renvoyer_code(State(state.clone()), Extension(session()), Path(id), local, Form(form(&[]))).await.unwrap();
        let envois = parity::envois_de_test();
        // Le code part quand même, mais sans lien local inutilisable chez le client.
        assert!(envois.iter().all(|e| !e.2.contains("192.168.1.10")));
        assert!(envois.iter().any(|e| e.2.contains("Code") || e.2.contains("code")));
    }

    #[tokio::test]
    async fn annonce_exige_confirmation_et_respecte_les_desinscrits() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        sqlx::raw_sql(
            "INSERT INTO clientventedirecte(id,nom,email,newsletter_email,token_desinscription) VALUES
               (10,'Accord','accord@example.fr',1,'t10'),(11,'Ancien','ancien@example.fr',0,'t11'),(12,'Parti','parti@example.fr',0,'t12');
             UPDATE clientventedirecte SET desinscrit_le=CURRENT_TIMESTAMP WHERE id=12;
             INSERT INTO commandeventedirecte(client_id,nom_client,telephone,statut) VALUES(11,'Ancien','06','livree'),(12,'Parti','06','livree');",
        )
        .execute(&state.pool)
        .await
        .unwrap();
        assert_eq!(destinataires_annonce(&state.pool, false).await.unwrap().len(), 1);
        assert_eq!(destinataires_annonce(&state.pool, true).await.unwrap().len(), 2);

        parity::vider_envois_de_test();
        let base = [("sujet", "Commandes ouvertes"), ("message", "Bonjour"), ("anciens", "1"), ("nb_attendu", "2")];
        // Sans la case cochée : rien ne part.
        assert!(annonce_envoyer(State(state.clone()), Extension(session()), headers_publics(), Form(form(&base))).await.is_err());
        // Nombre affiché différent : rien ne part.
        let mut faux = base.to_vec();
        faux[3] = ("nb_attendu", "5");
        faux.push(("confirmer", "1"));
        assert!(annonce_envoyer(State(state.clone()), Extension(session()), headers_publics(), Form(form(&faux))).await.is_err());
        assert!(parity::envois_de_test().is_empty());

        let mut ok = base.to_vec();
        ok.push(("confirmer", "1"));
        annonce_envoyer(State(state.clone()), Extension(session()), headers_publics(), Form(form(&ok))).await.unwrap();
        let envois = parity::envois_de_test();
        let mut to: Vec<_> = envois.iter().map(|e| e.0.clone()).collect();
        to.sort();
        assert_eq!(to, vec!["accord@example.fr", "ancien@example.fr"]);
        assert!(envois.iter().all(|e| e.2.contains("https://elevage.example.fr/commande")));
        assert!(envois.iter().any(|e| e.2.contains("/desinscription/t11")));

        // Désinscription : le client ne reçoit plus rien.
        let _ = parity::desinscription(State(state.clone()), Path("t11".into())).await.unwrap();
        assert_eq!(destinataires_annonce(&state.pool, true).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn case_prevenez_moi_enregistre_l_accord() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        commander(&state, &[("nom_client", "Alice"), ("telephone", "06"), ("email", "alice@example.fr"), ("q_1", "1"), ("prevenir", "1")]).await;
        commander(&state, &[("nom_client", "Bob"), ("telephone", "06"), ("email", "bob@example.fr"), ("q_1", "1")]).await;
        let accords: Vec<(String, i64)> = sqlx::query_as("SELECT email,newsletter_email FROM clientventedirecte ORDER BY email").fetch_all(&state.pool).await.unwrap();
        assert_eq!(accords, vec![("alice@example.fr".into(), 1), ("bob@example.fr".into(), 0)]);
    }

    async fn ordre(state: &AppState) -> Vec<i64> {
        sqlx::query_scalar("SELECT id FROM produitventedirecte ORDER BY ordre, id").fetch_all(&state.pool).await.unwrap()
    }

    #[tokio::test]
    async fn produit_deplace_a_la_place_choisie() {
        let state = vente().await;
        let deplacer = |id: i64, champs: &'static [(&'static str, &'static str)]| {
            let state = state.clone();
            async move { produit_deplacer(State(state), Extension(session()), Path(id), Form(form(champs))).await.unwrap() }
        };
        deplacer(3, &[("position", "1")]).await;
        assert_eq!(ordre(&state).await, vec![3, 1, 2]);
        deplacer(3, &[("direction", "dernier")]).await;
        assert_eq!(ordre(&state).await, vec![1, 2, 3]);
        deplacer(2, &[("direction", "haut")]).await;
        assert_eq!(ordre(&state).await, vec![2, 1, 3]);
        deplacer(1, &[("position", "99")]).await;
        assert_eq!(ordre(&state).await, vec![2, 3, 1]);
        // Glisser-déposer : ordre complet, identifiants inconnus ignorés, oubliés remis à la fin.
        produits_ordonner(State(state.clone()), Extension(session()), Form(form(&[("ordre", "1,42,3")]))).await.unwrap();
        assert_eq!(ordre(&state).await, vec![1, 3, 2]);
    }

    #[test]
    fn adresses_saisies_a_la_main() {
        let (bonnes, mauvaises) = lire_adresses("Marie@Exemple.fr\nJean Martin <jean@exemple.fr>; paul@exemple.fr, marie@exemple.fr\n\npas-une-adresse");
        assert_eq!(
            bonnes,
            vec![
                (String::new(), "marie@exemple.fr".to_string()),
                ("Jean Martin".to_string(), "jean@exemple.fr".to_string()),
                (String::new(), "paul@exemple.fr".to_string()),
            ]
        );
        assert_eq!(mauvaises, vec!["pas-une-adresse".to_string()]);
        assert_eq!(lire_adresses("a@b").1, vec!["a@b".to_string()]);
    }

    #[tokio::test]
    async fn annonce_ajout_d_adresses_et_texte_enregistre() {
        let state = vente().await;
        ouvrir_vente(&state).await;
        sqlx::raw_sql("INSERT INTO clientventedirecte(id,nom,email,newsletter_email,newsletter_sms,cree_le,token_desinscription,desinscrit_le) VALUES(10,'Ancien','ancien@example.fr',0,0,CURRENT_TIMESTAMP,'t10',CURRENT_TIMESTAMP)")
            .execute(&state.pool)
            .await
            .unwrap();
        // Une adresse invalide : rien n'est ajouté.
        assert!(annonce_ajouter_adresses(State(state.clone()), Extension(session()), Form(form(&[("adresses", "neuf@example.fr\nfaux")]))).await.is_err());
        assert!(destinataires_annonce(&state.pool, false).await.unwrap().is_empty());
        annonce_ajouter_adresses(State(state.clone()), Extension(session()), Form(form(&[("adresses", "Nouveau Client <neuf@example.fr>\nANCIEN@example.fr")]))).await.unwrap();
        let liste = destinataires_annonce(&state.pool, false).await.unwrap();
        let emails: Vec<_> = liste.iter().map(|d| (d.1.as_str(), d.2.as_str())).collect();
        assert_eq!(emails, vec![("Ancien", "ancien@example.fr"), ("Nouveau Client", "neuf@example.fr")]);
        // Retirer : sort de la liste, même avec les anciens clients.
        annonce_retirer(State(state.clone()), Extension(session()), Path(10), Form(form(&[]))).await.unwrap();
        assert_eq!(destinataires_annonce(&state.pool, true).await.unwrap().len(), 1);

        // Texte modifié enregistré puis repris sur la page.
        annonce_enregistrer(State(state.clone()), Extension(session()), Form(form(&[("sujet", "Porc fermier dispo"), ("message", "Mon texte à moi")]))).await.unwrap();
        let page = annonce_page(State(state.clone()), Extension(session()), Query(HashMap::new()), headers_publics()).await.unwrap().0;
        assert!(page.contains("Porc fermier dispo") && page.contains("Mon texte à moi"));
        annonce_enregistrer(State(state.clone()), Extension(session()), Form(form(&[("reinitialiser", "1")]))).await.unwrap();
        let page = annonce_page(State(state.clone()), Extension(session()), Query(HashMap::new()), headers_publics()).await.unwrap().0;
        assert!(!page.contains("Mon texte à moi") && page.contains("Les commandes sont ouvertes"));
    }
}

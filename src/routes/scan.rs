//! Entrée de pharmacie par scan de l'emballage.
//!
//! Le téléphone décode l'image et n'envoie ici que le texte du code. Le
//! serveur l'interprète, retrouve le produit s'il connaît déjà ce code, et
//! prépare un mouvement d'entrée que l'éleveur valide après relecture.
//!
//! Le parcours est volontairement en deux temps — analyser, puis enregistrer.
//! Un scan ne doit jamais écrire tout seul dans le stock : une boîte mal
//! cadrée, un code voisin lu par erreur, et l'inventaire devient faux sans que
//! personne ne s'en aperçoive. La relecture à l'écran est ce qui rend l'erreur
//! visible avant qu'elle ne coûte quelque chose.

use super::*;
use crate::gs1;

fn annee_courante() -> i32 {
    use chrono::Datelike;
    Local::now().year()
}

/// Analyse un code scanné et dit ce qu'on en sait.
///
/// N'écrit rien, hormis la trace du scan : cette route est appelée à chaque
/// lecture, y compris les lectures ratées.
pub(super) async fn analyser(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let brut =
        form_text(&form, "code").ok_or_else(|| AppError::Invalid("Aucun code transmis".into()))?;
    let lu = gs1::decoder(&brut, annee_courante());
    let cle = lu.cle();

    let connu: Option<(i64, String, f64, Option<String>)> = sqlx::query_as(
        "SELECT id,produit,unites_par_boite,libelle_fabricant FROM code_produit \
         WHERE cle=? AND domaine='pharmacie'",
    )
    .bind(&cle)
    .fetch_optional(&state.pool)
    .await?;

    sqlx::query(
        "INSERT INTO scan_journal(utilisateur,domaine,code_brut,gtin,lot,peremption,reconnu,resultat) \
         VALUES(?,'pharmacie',?,?,?,?,?,?)",
    )
    .bind(&session.identifiant)
    .bind(&brut)
    .bind(lu.gtin.clone())
    .bind(lu.lot.clone())
    .bind(lu.peremption.clone())
    .bind(i64::from(connu.is_some()))
    .bind(if connu.is_some() {
        "produit reconnu"
    } else if lu.gs1 {
        "code lu, produit inconnu"
    } else {
        "code non interprété"
    })
    .execute(&state.pool)
    .await?;

    // Liste des produits existants, pour l'association quand le code est
    // inconnu. Elle est courte : une pharmacie d'élevage tient sur un écran.
    let produits: Vec<String> =
        sqlx::query_scalar("SELECT produit FROM produitpharmacie ORDER BY produit")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();

    let corps = json!({
        "brut": lu.brut,
        "cle": cle,
        "gs1": lu.gs1,
        "gtin": lu.gtin,
        "lot": lu.lot,
        "peremption": lu.peremption,
        "serie": lu.serie,
        "tracabilite": lu.a_tracabilite(),
        "connu": connu.is_some(),
        "produit": connu.as_ref().map(|(_, produit, _, _)| produit.clone()),
        "unites_par_boite": connu.as_ref().map(|(_, _, unites, _)| *unites).unwrap_or(1.0),
        "libelle_fabricant": connu.as_ref().and_then(|(_, _, _, libelle)| libelle.clone()),
        "produits": produits,
        "message": if connu.is_some() {
            "Produit reconnu."
        } else if lu.gs1 {
            "Code lu, mais ce produit n'est pas encore associé. Indiquez de quoi il s'agit : ce sera retenu."
        } else if lu.brut.is_empty() {
            "Code vide."
        } else {
            "Code non reconnu comme un code d'emballage. Il peut quand même être associé à un produit."
        },
    });
    Ok((
        [(header::CONTENT_TYPE, "application/json")],
        corps.to_string(),
    )
        .into_response())
}

/// Valide l'entrée en stock, et retient l'association si elle est nouvelle.
pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;

    let produit = form_text(&form, "produit")
        .ok_or_else(|| AppError::Invalid("Indiquez le produit".into()))?;
    let cle = form_text(&form, "cle").unwrap_or_default();
    let brut = form_text(&form, "code").unwrap_or_else(|| cle.clone());
    let lot = form_text(&form, "lot");
    let peremption = form_text(&form, "peremption");
    let boites = form_f64(&form, "boites").unwrap_or(1.0);
    let unites_par_boite = form_f64(&form, "unites_par_boite").unwrap_or(1.0);

    if boites <= 0.0 || unites_par_boite <= 0.0 {
        return Err(AppError::Invalid(
            "Le nombre de boîtes et le conditionnement doivent être supérieurs à zéro".into(),
        ));
    }
    let quantite = boites * unites_par_boite;

    // Une péremption déjà passée est presque toujours une erreur de lecture ou
    // de siècle. On refuse plutôt que d'empoisonner le registre : l'éleveur
    // corrige le champ, qui reste modifiable.
    if let Some(date) = peremption.as_deref() {
        if let Some(jour) = parse_stored_date(date) {
            if jour < Local::now().date_naive() {
                return Err(AppError::Invalid(format!(
                    "La péremption lue ({date}) est déjà passée. Vérifiez la boîte et corrigez la date."
                )));
            }
        }
    }

    let mut transaction = state.pool.begin().await?;

    // Le produit doit exister en pharmacie : on le crée au besoin, sinon le
    // mouvement porterait sur un stock fantôme.
    sqlx::query(
        "INSERT INTO produitpharmacie(produit,stock_actuel,unite,maj) \
         SELECT ?,0,'doses',CURRENT_TIMESTAMP \
         WHERE NOT EXISTS(SELECT 1 FROM produitpharmacie WHERE lower(produit)=lower(?))",
    )
    .bind(&produit)
    .bind(&produit)
    .execute(&mut *transaction)
    .await?;

    sqlx::query(
        "UPDATE produitpharmacie SET stock_actuel=COALESCE(stock_actuel,0)+?,maj=CURRENT_TIMESTAMP \
         WHERE lower(produit)=lower(?)",
    )
    .bind(quantite)
    .bind(&produit)
    .execute(&mut *transaction)
    .await?;

    let note = format!(
        "Entrée par scan : {boites} boîte(s) de {unites_par_boite}{}",
        lot.as_deref()
            .map(|lot| format!(", lot {lot}"))
            .unwrap_or_default()
    );
    sqlx::query(
        "INSERT INTO mouvementpharmacie(produit,date,type,quantite,note,lot,peremption,code_scanne) \
         VALUES(?,date('now','localtime'),'entree',?,?,?,?,?)",
    )
    .bind(&produit)
    .bind(quantite)
    .bind(note)
    .bind(lot.clone())
    .bind(peremption.clone())
    .bind(&brut)
    .execute(&mut *transaction)
    .await?;

    // Apprentissage : la prochaine boîte du même produit sera reconnue seule.
    if !cle.is_empty() {
        sqlx::query(
            "INSERT INTO code_produit(cle,domaine,produit,unites_par_boite,libelle_fabricant,dernier_scan,nb_scans) \
             VALUES(?,'pharmacie',?,?,?,datetime('now'),1) \
             ON CONFLICT(cle) DO UPDATE SET produit=excluded.produit, \
               unites_par_boite=excluded.unites_par_boite, \
               dernier_scan=datetime('now'), nb_scans=nb_scans+1",
        )
        .bind(&cle)
        .bind(&produit)
        .bind(unites_par_boite)
        .bind(form_text(&form, "libelle_fabricant"))
        .execute(&mut *transaction)
        .await?;
    }

    transaction.commit().await?;

    let message = format!("{quantite} {produit} ajouté(s) au stock");
    Ok(Redirect::to(&format!("/pharmacie/scan?ok={}", encode_query(&message))).into_response())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let produits: Vec<String> =
        sqlx::query_scalar("SELECT produit FROM produitpharmacie ORDER BY produit")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    let derniers = generic_rows(
        &state.pool,
        "SELECT m.date, m.produit, m.quantite, m.lot, m.peremption \
         FROM mouvementpharmacie m WHERE m.code_scanne IS NOT NULL \
         ORDER BY m.id DESC LIMIT 10",
    )
    .await?;
    let associations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM code_produit WHERE domaine='pharmacie'")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);

    let mut contexte = context(&session);
    contexte.insert(
        "produits".into(),
        Value::Array(produits.into_iter().map(Value::String).collect()),
    );
    contexte.insert("derniers".into(), Value::Array(derniers));
    contexte.insert("associations".into(), json!(associations));
    contexte.insert(
        "ok".into(),
        match parametres.get("ok") {
            Some(message) => Value::String(message.replace('+', " ")),
            None => Value::Null,
        },
    );
    render(&state, "pharmacie_scan.html", Value::Object(contexte))
}

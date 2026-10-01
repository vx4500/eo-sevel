//! Pilotage à distance de la machine à soupe par le service SOAP de
//! Mega-feeder (« Communications externes », port 251 sur le PC de la VM 101).
//! Mêmes appels que la web-app Asserva (www/index.html du disque Exafeeder) :
//! lectures GetSynoptique, ListeCycleEffectueGetList, ConsigneRapideGetList,
//! FicheVanneGetConsigne/GetControle ; écritures FicheVannePutConsigne,
//! AcquitterAlarme, EntreeSortieValidation — chacune confirmée et journalisée.

use super::*;

const URL_DEFAUT: &str = "http://192.168.1.195:251/soap/IExafeeder";

/// Champs d'une fiche vanne, dans l'ordre de la web-app Asserva.
const CHAMPS_FICHE: [(&str, &str); 13] = [
    ("VanneValidation", "Validation fiche"),
    ("VanneNombrePresent", "Nombre présent"),
    ("VanneNumService", "N° service / formule"),
    ("VanneNumPlan", "Plan"),
    ("VannePourcentageRation", "% ration"),
    ("VanneJourRetourPlan", "Jour retour au plan"),
    ("VanneCodeEau", "Code eau"),
    ("VanneQuantiteeEaux", "Quantité eau"),
    ("VanneTempsArret", "Temps d'arrêt"),
    ("VanneNumGroupe", "N° groupe"),
    ("VanneNumLot", "N° lot"),
    ("VanneReductionSonde", "% réduction sonde"),
    ("VanneJourPlan", "Jour du plan"),
];

async fn url(pool: &SqlitePool) -> String {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle='soupe_soap_url'")
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .unwrap_or_else(|| URL_DEFAUT.into())
}

/// Un appel SOAP ; `corps` est l'élément du Body (valeurs déjà validées).
async fn appel(pool: &SqlitePool, corps: &str) -> Result<String, String> {
    let enveloppe = format!(r#"<Envelope xmlns="http://schemas.xmlsoap.org/soap/envelope/"><Body>{corps}</Body></Envelope>"#);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let reponse = client
        .post(url(pool).await)
        .header("Content-Type", "text/xml")
        .body(enveloppe)
        .send()
        .await
        .map_err(|_| "Mega-feeder ne répond pas (PC éteint, programme fermé ou port 251 bloqué)".to_string())?;
    let texte = reponse.text().await.map_err(|e| e.to_string())?;
    if texte.contains("Fault>") {
        return Err(format!("Mega-feeder a refusé la demande : {}", valeur(&texte, "faultstring").unwrap_or_default()));
    }
    Ok(texte)
}

fn valeur(xml: &str, balise: &str) -> Option<String> {
    let debut = xml.find(&format!("<{balise}>"))? + balise.len() + 2;
    let fin = xml[debut..].find(&format!("</{balise}>"))? + debut;
    Some(xml[debut..fin].trim().to_string())
}

fn items(xml: &str, balises: &[&str]) -> Vec<Value> {
    xml.split("<item>")
        .skip(1)
        .map(|bloc| {
            let mut o = Map::new();
            for b in balises {
                o.insert((*b).into(), json!(valeur(bloc, b).unwrap_or_default()));
            }
            Value::Object(o)
        })
        .collect()
}

fn fiche(xml: &str) -> Value {
    let mut o = Map::new();
    for (b, _) in CHAMPS_FICHE {
        o.insert(b.into(), json!(valeur(xml, b).unwrap_or_default()));
    }
    Value::Object(o)
}

fn entier(form: &HashMap<String, String>, cle: &str, min: i64, max: i64) -> AppResult<i64> {
    form.get(cle)
        .and_then(|v| v.trim().parse::<i64>().ok())
        .filter(|v| (min..=max).contains(v))
        .ok_or_else(|| AppError::Invalid(format!("Valeur invalide pour « {cle} » ({min} à {max})")))
}

fn confirme(form: &HashMap<String, String>) -> AppResult<()> {
    if form.get("confirme").map(String::as_str) == Some("1") {
        Ok(())
    } else {
        Err(AppError::Invalid("Cochez la case de confirmation avant d'envoyer à la machine.".into()))
    }
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let mut ctx = context(&session);
    let mut erreur = None;
    match appel(pool, r#"<GetSynoptique xmlns="http://tempuri.org/"><autre>0</autre></GetSynoptique>"#).await {
        Ok(x) => {
            ctx.insert("synoptique".into(), json!({
                "etat": valeur(&x, "EtatAutomate"), "cuve1": valeur(&x, "PoidCuve1"), "cuve2": valeur(&x, "PoidCuve2"),
                "sequence": valeur(&x, "NumSequence"), "formule": valeur(&x, "Numservice"), "alarme": valeur(&x, "Alarme").filter(|a| !a.is_empty()),
            }));
        }
        Err(e) => erreur = Some(e),
    }
    if erreur.is_none() {
        if let Ok(x) = appel(pool, r#"<ListeCycleEffectueGetList xmlns="http://tempuri.org/"><Reponse></Reponse></ListeCycleEffectueGetList>"#).await {
            ctx.insert("cycles".into(), Value::Array(items(&x, &["Sequence", "Formule", "Heure", "Effectue", "NomFormule"])));
        }
        if let Ok(x) = appel(pool, r#"<ConsigneRapideGetList xmlns="http://tempuri.org/"><Reponse></Reponse></ConsigneRapideGetList>"#).await {
            ctx.insert("vannes".into(), Value::Array(items(&x, &["ConsigneRapideNumeroVanne", "ConsigneRapideNombreAnimaux", "ConsigneRapideModulationRation", "ConsigneRapideRation", "ConsigneRapideNumeroSalle"])));
        }
    }
    ctx.insert("erreur".into(), json!(erreur));
    ctx.insert("url".into(), json!(url(pool).await));
    ctx.insert("ok".into(), json!(query.get("ok")));
    render(&state, "soupe_pilotage.html", Value::Object(ctx))
}

pub(super) async fn vanne(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(n): Path<i64>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    if !(1..=999).contains(&n) {
        return Err(AppError::NotFound);
    }
    let pool = &state.pool;
    let mut ctx = context(&session);
    match appel(pool, &format!(r#"<FicheVanneGetConsigne xmlns="http://tempuri.org/"><Vanne>{n}</Vanne><Reponse></Reponse></FicheVanneGetConsigne>"#)).await {
        Ok(x) => {
            ctx.insert("fiche".into(), fiche(&x));
        }
        Err(e) => {
            ctx.insert("erreur".into(), json!(e));
        }
    }
    if let Ok(x) = appel(pool, &format!(r#"<FicheVanneGetControle xmlns="http://tempuri.org/"><Vanne>{n}</Vanne><Reponse></Reponse></FicheVanneGetControle>"#)).await {
        ctx.insert("controle".into(), json!({
            "age": valeur(&x, "VanneAgeVie"),
            "cumul": valeur(&x, "VanneCumulKgSoupe").and_then(|v| v.replace(',', ".").parse::<f64>().ok()).map(|v| (v * 10.0).round() / 10.0),
            "ration": valeur(&x, "VanneRationActuelJour"),
        }));
    }
    ctx.insert("n".into(), json!(n));
    ctx.insert("champs".into(), json!(CHAMPS_FICHE.iter().map(|(c, l)| json!({"cle": c, "libelle": l})).collect::<Vec<_>>()));
    ctx.insert("ok".into(), json!(query.get("ok")));
    render(&state, "soupe_vanne.html", Value::Object(ctx))
}

/// Animaux présents et % ration (la « Config rapide » d'Asserva).
pub(super) async fn rapide(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(n): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    confirme(&form)?;
    let animaux = entier(&form, "animaux", 0, 999)?;
    let ration = entier(&form, "ration", 0, 300)?;
    let avant = appel(&state.pool, &format!(r#"<FicheVanneGetConsigne xmlns="http://tempuri.org/"><Vanne>{n}</Vanne><Reponse></Reponse></FicheVanneGetConsigne>"#)).await.map_err(AppError::Invalid)?;
    appel(&state.pool, &format!(r#"<FicheVannePutConsigne xmlns="http://tempuri.org/"><Consigne><VanneNombrePresent>{animaux}</VanneNombrePresent><VannePourcentageRation>{ration}</VannePourcentageRation></Consigne><NumVanne>{n}</NumVanne></FicheVannePutConsigne>"#)).await.map_err(AppError::Invalid)?;
    db::journal(&state.pool, &session.nom, "soupe modifier", "vanne",
        &format!("vanne {n} : animaux {}→{animaux}, % ration {}→{ration}", valeur(&avant, "VanneNombrePresent").unwrap_or_default(), valeur(&avant, "VannePourcentageRation").unwrap_or_default()),
        "/equipements/soupe/pilotage").await;
    Ok(Redirect::to(&format!("/equipements/soupe/pilotage?ok=Vanne+{n}+modifi%C3%A9e#vanne-{n}")).into_response())
}

/// Fiche vanne complète.
pub(super) async fn fiche_maj(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(n): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    confirme(&form)?;
    let avant = appel(&state.pool, &format!(r#"<FicheVanneGetConsigne xmlns="http://tempuri.org/"><Vanne>{n}</Vanne><Reponse></Reponse></FicheVanneGetConsigne>"#)).await.map_err(AppError::Invalid)?;
    let mut xml = String::new();
    let mut changements = Vec::new();
    for (cle, libelle) in CHAMPS_FICHE {
        let v = entier(&form, cle, 0, 99999)?;
        let ancien = valeur(&avant, cle).unwrap_or_default();
        if ancien != v.to_string() {
            changements.push(format!("{libelle} {ancien}→{v}"));
        }
        xml.push_str(&format!("<{cle}>{v}</{cle}>"));
    }
    if changements.is_empty() {
        return Ok(Redirect::to(&format!("/equipements/soupe/vanne/{n}?ok=Aucun+changement")).into_response());
    }
    appel(&state.pool, &format!(r#"<FicheVannePutConsigne xmlns="http://tempuri.org/"><Consigne>{xml}</Consigne><NumVanne>{n}</NumVanne></FicheVannePutConsigne>"#)).await.map_err(AppError::Invalid)?;
    db::journal(&state.pool, &session.nom, "soupe modifier", "vanne", &format!("vanne {n} : {}", changements.join(", ")), "/equipements/soupe/vanne").await;
    Ok(Redirect::to(&format!("/equipements/soupe/vanne/{n}?ok=Fiche+envoy%C3%A9e+%C3%A0+la+machine")).into_response())
}

pub(super) async fn alarme(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    confirme(&form)?;
    let x = appel(&state.pool, r#"<GetSynoptique xmlns="http://tempuri.org/"><autre>0</autre></GetSynoptique>"#).await.map_err(AppError::Invalid)?;
    let alarme = valeur(&x, "Alarme").unwrap_or_default();
    appel(&state.pool, r#"<AcquitterAlarme xmlns="http://tempuri.org/"><autre>0</autre></AcquitterAlarme>"#).await.map_err(AppError::Invalid)?;
    db::journal(&state.pool, &session.nom, "soupe acquitter", "alarme", &format!("alarme acquittée : {alarme}"), "/equipements/soupe/pilotage").await;
    Ok(Redirect::to("/equipements/soupe/pilotage?ok=Alarme+acquitt%C3%A9e").into_response())
}

pub(super) async fn mouvement(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    confirme(&form)?;
    let action = match form.get("action").map(String::as_str) {
        Some(a @ ("ActionEntree" | "ActionSortie" | "ActionVente" | "ActionPerte")) => a.to_string(),
        _ => return Err(AppError::Invalid("Action invalide".into())),
    };
    let vanne = entier(&form, "vanne", 1, 999)?;
    let nombre = entier(&form, "nombre", 1, 999)?;
    let poids = entier(&form, "poids_total", 0, 99999)?;
    let x = appel(&state.pool, &format!(r#"<EntreeSortieValidation xmlns="http://tempuri.org/"><Action>{action}</Action><NumVanne>{vanne}</NumVanne><Nombre>{nombre}</Nombre><PoidTotal>{poids}</PoidTotal></EntreeSortieValidation>"#)).await.map_err(AppError::Invalid)?;
    let retour = valeur(&x, "return").unwrap_or_default();
    if retour != "OK" {
        return Err(AppError::Invalid(format!("Mega-feeder a répondu : {retour}")));
    }
    db::journal(&state.pool, &session.nom, "soupe mouvement", "vanne", &format!("{action} vanne {vanne} : {nombre} animaux, {poids} kg"), "/equipements/soupe/pilotage").await;
    Ok(Redirect::to("/equipements/soupe/pilotage?ok=Mouvement+enregistr%C3%A9+dans+Mega-feeder").into_response())
}

pub(super) async fn regler(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let u = form.get("url").map(|u| u.trim().to_string()).filter(|u| u.starts_with("http://") || u.starts_with("https://"))
        .ok_or_else(|| AppError::Invalid("Adresse invalide (http://…)".into()))?;
    sqlx::query("INSERT INTO parametre(cle,valeur) VALUES('soupe_soap_url',?) ON CONFLICT(cle) DO UPDATE SET valeur=excluded.valeur")
        .bind(u).execute(&state.pool).await?;
    Ok(Redirect::to("/equipements/soupe/pilotage").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lecture_des_reponses_reelles() {
        let x = "<return><item><ConsigneRapideNumeroVanne>6</ConsigneRapideNumeroVanne><ConsigneRapideModulationRation>105</ConsigneRapideModulationRation><ConsigneRapideNombreAnimaux>16</ConsigneRapideNombreAnimaux></item><item><ConsigneRapideNumeroVanne>7</ConsigneRapideNumeroVanne></item></return>";
        let v = items(x, &["ConsigneRapideNumeroVanne", "ConsigneRapideNombreAnimaux"]);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0]["ConsigneRapideNombreAnimaux"], "16");
        assert_eq!(valeur("<a><Alarme></Alarme></a>", "Alarme").as_deref(), Some(""));
    }
}

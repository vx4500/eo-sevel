//! Mes équipements → Caméras : caméras IP rattachées aux salles.
//!
//! Beaucoup de caméras génériques (serveur « LSWebServer » de la maternité)
//! n'ont pas d'adresse de photo : l'image est donc tirée du flux RTSP par
//! ffmpeg, seulement quand quelqu'un regarde. Une capture à la fois par
//! caméra, image réutilisée quelques secondes, rien sur le disque. Le mot de
//! passe reste sur le serveur : le navigateur ne voit que /salles/camera/{id}/image.

use super::*;
use std::sync::LazyLock;
use std::time::{Duration as StdDuration, Instant};
use tokio::sync::Mutex;

/// Durée pendant laquelle une image est resservie sans réinterroger la caméra.
const FRAICHEUR: StdDuration = StdDuration::from_secs(2);
/// Au-delà, la caméra est considérée injoignable.
const DELAI_CAPTURE: StdDuration = StdDuration::from_secs(10);

type Capture = Arc<Mutex<Option<(Instant, Vec<u8>)>>>;
static CAPTURES: LazyLock<DashMap<i64, Capture>> = LazyLock::new(DashMap::new);

/// Retire d'une adresse `rtsp://user:pass@hote/...` les identifiants qu'on y
/// aurait collés, pour les ranger à part.
fn separer(url: &str) -> (String, Option<String>, Option<String>) {
    let url = url.trim();
    let Some((schema, reste)) = url.split_once("://") else {
        return (url.to_string(), None, None);
    };
    let fin_hote = reste.find('/').unwrap_or(reste.len());
    let Some(arobase) = reste[..fin_hote].rfind('@') else {
        return (url.to_string(), None, None);
    };
    let (user, pass) = match reste[..arobase].split_once(':') {
        Some((u, p)) => (u.to_string(), Some(p.to_string())),
        None => (reste[..arobase].to_string(), None),
    };
    (format!("{schema}://{}", &reste[arobase + 1..]), Some(user).filter(|u| !u.is_empty()), pass)
}

fn encoder(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Adresse complète passée à ffmpeg, identifiants inclus.
fn url_complete(url: &str, user: Option<&str>, pass: Option<&str>) -> String {
    let Some(user) = user.filter(|u| !u.is_empty()) else {
        return url.to_string();
    };
    let Some((schema, reste)) = url.split_once("://") else {
        return url.to_string();
    };
    let identifiants = match pass.filter(|p| !p.is_empty()) {
        Some(p) => format!("{}:{}", encoder(user), encoder(p)),
        None => encoder(user),
    };
    format!("{schema}://{identifiants}@{reste}")
}

async fn capturer(url: &str, secret: Option<&str>) -> Result<Vec<u8>, String> {
    let mut commande = tokio::process::Command::new("ffmpeg");
    commande.kill_on_drop(true);
    if url.starts_with("rtsp") {
        commande.args(["-rtsp_transport", "tcp"]);
    }
    commande
        .args(["-loglevel", "error", "-nostdin", "-i", url, "-frames:v", "1", "-f", "image2", "-c:v", "mjpeg", "-q:v", "5", "pipe:1"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let enfant = commande.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "ffmpeg n'est pas installé sur le serveur (apt install ffmpeg)".to_string()
        } else {
            format!("ffmpeg ne démarre pas : {e}")
        }
    })?;
    let sortie = tokio::time::timeout(DELAI_CAPTURE, enfant.wait_with_output())
        .await
        .map_err(|_| "la caméra ne répond pas".to_string())?
        .map_err(|e| format!("ffmpeg : {e}"))?;
    if sortie.status.success() && !sortie.stdout.is_empty() {
        return Ok(sortie.stdout);
    }
    let mut message = String::from_utf8_lossy(&sortie.stderr).lines().last().unwrap_or("aucune image reçue").to_string();
    // ffmpeg recopie l'adresse dans ses erreurs : ne jamais y laisser le mot de passe.
    if let Some(secret) = secret.filter(|s| !s.is_empty()) {
        message = message.replace(&encoder(secret), "***").replace(secret, "***");
    }
    Err(message)
}

pub(super) async fn image(State(state): State<AppState>, Path(id): Path<i64>) -> AppResult<Response> {
    let camera: Option<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT url, identifiant, mot_de_passe FROM camera WHERE id=? AND actif=1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let (url, user, pass) = camera.ok_or(AppError::NotFound)?;
    let verrou = CAPTURES.entry(id).or_default().clone();
    let mut derniere = verrou.lock().await;
    let jpeg = match derniere.as_ref() {
        Some((prise, jpeg)) if prise.elapsed() < FRAICHEUR => jpeg.clone(),
        _ => match capturer(&url_complete(&url, user.as_deref(), pass.as_deref()), pass.as_deref()).await {
            Ok(jpeg) => {
                *derniere = Some((Instant::now(), jpeg.clone()));
                jpeg
            }
            Err(message) => {
                tracing::warn!(camera = id, %message, "image de caméra indisponible");
                return Ok((StatusCode::BAD_GATEWAY, [(header::CACHE_CONTROL, "no-store")], format!("Caméra injoignable : {message}")).into_response());
            }
        },
    };
    Ok(([(header::CONTENT_TYPE, "image/jpeg"), (header::CACHE_CONTROL, "no-store")], jpeg).into_response())
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let mut ctx = context(&session);
    ctx.insert(
        "cameras".into(),
        Value::Array(
            generic_rows(
                pool,
                "SELECT c.id, c.nom, c.url, c.identifiant, \
                 (c.mot_de_passe IS NOT NULL AND c.mot_de_passe<>'') AS mdp, c.salle_id, c.actif, s.nom AS salle \
                 FROM camera c LEFT JOIN salle s ON s.id=c.salle_id ORDER BY s.ordre, s.nom, c.nom",
            )
            .await?,
        ),
    );
    ctx.insert("salles".into(), Value::Array(generic_rows(pool, "SELECT id, nom FROM salle ORDER BY ordre, nom").await?));
    ctx.insert("resultat".into(), json!(query.get("resultat")));
    render(&state, "cameras.html", Value::Object(ctx))
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Donnez un nom à la caméra".into()))?;
    let (url, user_url, pass_url) = separer(&form_text(&form, "url").unwrap_or_default());
    if !["rtsp://", "rtsps://", "http://", "https://"].iter().any(|s| url.starts_with(s)) {
        return Err(AppError::Invalid("L'adresse du flux commence par rtsp:// (ou http://)".into()));
    }
    let user = form_text(&form, "identifiant").or(user_url);
    // Mot de passe laissé vide : on garde celui déjà enregistré.
    let pass = form_text(&form, "mot_de_passe").or(pass_url);
    let salle_id = form.get("salle_id").and_then(|v| v.parse::<i64>().ok());
    let actif = i64::from(form.contains_key("actif"));
    let id = form.get("id").and_then(|v| v.parse::<i64>().ok());
    let id = match id {
        Some(id) => {
            sqlx::query("UPDATE camera SET nom=?, url=?, identifiant=?, mot_de_passe=COALESCE(?, mot_de_passe), salle_id=?, actif=? WHERE id=?")
                .bind(&nom)
                .bind(&url)
                .bind(&user)
                .bind(&pass)
                .bind(salle_id)
                .bind(actif)
                .bind(id)
                .execute(pool)
                .await?;
            id
        }
        None => sqlx::query("INSERT INTO camera (nom, url, identifiant, mot_de_passe, salle_id, actif) VALUES (?,?,?,?,?,?)")
            .bind(&nom)
            .bind(&url)
            .bind(&user)
            .bind(&pass)
            .bind(salle_id)
            .bind(actif)
            .execute(pool)
            .await?
            .last_insert_rowid(),
    };
    CAPTURES.remove(&id);
    Ok(Redirect::to(&format!("/equipements/cameras?resultat=enregistre#camera-{id}")).into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    if !session.est_admin() {
        return Err(AppError::Forbidden);
    }
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM camera WHERE id=?").bind(id).execute(&state.pool).await?;
    CAPTURES.remove(&id);
    Ok(Redirect::to("/equipements/cameras?resultat=supprimee").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_identifiants_colles_dans_l_adresse_sont_mis_a_part() {
        let (url, user, pass) = separer(" rtsp://admin:Mot@passe@192.168.1.2:554/Streaming/Channels/102 ");
        assert_eq!(url, "rtsp://192.168.1.2:554/Streaming/Channels/102");
        assert_eq!(user.as_deref(), Some("admin"));
        assert_eq!(pass.as_deref(), Some("Mot@passe"));
        let (url, user, pass) = separer("rtsp://192.168.1.2/flux?a=b@c");
        assert_eq!((url.as_str(), user, pass), ("rtsp://192.168.1.2/flux?a=b@c", None, None));
    }

    #[test]
    fn l_adresse_complete_encode_les_caracteres_speciaux() {
        assert_eq!(
            url_complete("rtsp://192.168.1.2:554/x", Some("admin"), Some("p@ss:1")),
            "rtsp://admin:p%40ss%3A1@192.168.1.2:554/x"
        );
        assert_eq!(url_complete("rtsp://h/x", None, Some("p")), "rtsp://h/x");
    }
}

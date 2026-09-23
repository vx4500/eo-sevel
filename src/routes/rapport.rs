//! Rapport d'élevage complet : chiffres clés de la période, actions à mener
//! dans les 48 heures, bandes, reproduction, maternité, mortalité, sanitaire,
//! stocks, ventes, énergie et températures par salle.
//!
//! Tout est lu dans la base : les mesures et l'état des salles sont écrits par
//! `eo-regul`, les échéances viennent des bandes, protocoles, tâches et
//! entretiens. La page n'écrit rien. Elle s'imprime (ou s'enregistre en PDF
//! depuis le navigateur) telle qu'affichée.

use super::*;
use chrono::TimeZone;

/// Au-delà, le démon est considéré comme arrêté : il écrit son battement de
/// cœur à chaque cycle (30 s par défaut).
const BATTEMENT_MAX_S: i64 = 5 * 60;
/// Au-delà, l'état d'une salle n'est plus tenu pour actuel.
const ETAT_SALLE_MAX_S: i64 = 30 * 60;
/// Horizon des actions à mener.
const HORIZON_JOURS: i64 = 2;
/// Retards plus anciens ignorés dans les soins de portée : au-delà, ce sont
/// des saisies jamais clôturées, pas des soins encore à faire.
const RETARD_SOINS_MAX_JOURS: i64 = 14;

#[derive(Default)]
struct Statistiques {
    moyenne: Option<f64>,
    mini: Option<f64>,
    maxi: Option<f64>,
    ts_mini: Option<i64>,
    ts_maxi: Option<i64>,
    n: i64,
}

impl Statistiques {
    fn json(&self) -> Value {
        json!({
            "moyenne": self.moyenne,
            "mini": self.mini,
            "maxi": self.maxi,
            "heure_mini": self.ts_mini.map(heure_locale),
            "heure_maxi": self.ts_maxi.map(heure_locale),
            "n": self.n,
        })
    }
}

fn heure_locale(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|moment| moment.format("%d/%m %Hh%M").to_string())
        .unwrap_or_default()
}

fn maintenant_ts() -> i64 {
    Local::now().timestamp()
}

/// Statistiques d'une grandeur depuis `depuis` (mesures brutes, en dixièmes).
async fn statistiques(
    pool: &SqlitePool,
    salle_id: i64,
    grandeur: &str,
    depuis: i64,
) -> AppResult<Statistiques> {
    let ligne = sqlx::query(
        "SELECT AVG(valeur)/10.0 AS moy,MIN(valeur)/10.0 AS mini,MAX(valeur)/10.0 AS maxi,COUNT(*) AS n,\
         (SELECT ts FROM ambiance_mesure WHERE salle_id=?1 AND grandeur=?2 AND ts>=?3 ORDER BY valeur ASC,ts DESC LIMIT 1) AS ts_mini,\
         (SELECT ts FROM ambiance_mesure WHERE salle_id=?1 AND grandeur=?2 AND ts>=?3 ORDER BY valeur DESC,ts DESC LIMIT 1) AS ts_maxi \
         FROM ambiance_mesure WHERE salle_id=?1 AND grandeur=?2 AND ts>=?3",
    )
    .bind(salle_id)
    .bind(grandeur)
    .bind(depuis)
    .fetch_one(pool)
    .await?;
    Ok(Statistiques {
        moyenne: ligne.try_get::<Option<f64>, _>("moy").ok().flatten(),
        mini: ligne.try_get::<Option<f64>, _>("mini").ok().flatten(),
        maxi: ligne.try_get::<Option<f64>, _>("maxi").ok().flatten(),
        ts_mini: ligne.try_get::<Option<i64>, _>("ts_mini").ok().flatten(),
        ts_maxi: ligne.try_get::<Option<i64>, _>("ts_maxi").ok().flatten(),
        n: ligne.try_get("n").unwrap_or(0),
    })
}

/// Part des relevés de température hors des seuils d'alerte de la salle.
async fn part_hors_seuils(
    pool: &SqlitePool,
    salle_id: i64,
    depuis: i64,
    seuil_min: Option<f64>,
    seuil_max: Option<f64>,
) -> AppResult<Option<f64>> {
    if seuil_min.is_none() && seuil_max.is_none() {
        return Ok(None);
    }
    let ligne = sqlx::query(
        "SELECT COUNT(*) AS n,\
         SUM(CASE WHEN (?3 IS NOT NULL AND valeur/10.0<=?3) OR (?4 IS NOT NULL AND valeur/10.0>=?4) THEN 1 ELSE 0 END) AS hors \
         FROM ambiance_mesure WHERE salle_id=?1 AND grandeur='t' AND ts>=?2",
    )
    .bind(salle_id)
    .bind(depuis)
    .bind(seuil_min)
    .bind(seuil_max)
    .fetch_one(pool)
    .await?;
    let n: i64 = ligne.try_get("n").unwrap_or(0);
    let hors: i64 = ligne.try_get("hors").unwrap_or(0);
    Ok((n > 0).then(|| 100.0 * hors as f64 / n as f64))
}

async fn salles(pool: &SqlitePool, maintenant: i64) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT a.salle_id,s.nom,a.pilote,a.t_alerte_min,a.t_alerte_max,\
         e.ts,e.t,e.h,e.consigne,e.ventil,e.mode,e.sonde_ok,e.message,e.jour_bande,b.code AS bande \
         FROM ambiance_salle a JOIN salle s ON s.id=a.salle_id \
         LEFT JOIN ambiance_etat e ON e.salle_id=a.salle_id LEFT JOIN bande b ON b.id=e.bande_id \
         WHERE a.actif=1 ORDER BY s.ordre,s.nom",
    )
    .fetch_all(pool)
    .await?;
    let depuis_24h = maintenant - 24 * 3600;
    let depuis_7j = maintenant - 7 * 24 * 3600;
    let mut sortie = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        let salle_id: i64 = ligne.try_get("salle_id")?;
        let seuil_min: Option<f64> = ligne
            .try_get::<Option<f64>, _>("t_alerte_min")
            .ok()
            .flatten();
        let seuil_max: Option<f64> = ligne
            .try_get::<Option<f64>, _>("t_alerte_max")
            .ok()
            .flatten();
        let ts: Option<i64> = ligne.try_get::<Option<i64>, _>("ts").ok().flatten();
        let t24 = statistiques(pool, salle_id, "t", depuis_24h).await?;
        let t7 = statistiques(pool, salle_id, "t", depuis_7j).await?;
        let c24 = statistiques(pool, salle_id, "consigne", depuis_24h).await?;
        let h24 = statistiques(pool, salle_id, "h", depuis_24h).await?;
        let ecart = match (t24.moyenne, c24.moyenne) {
            (Some(t), Some(c)) => Some(t - c),
            _ => None,
        };
        let alertes: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_alerte WHERE salle_id=? AND debut>=datetime(?,'unixepoch','localtime')",
        )
        .bind(salle_id)
        .bind(depuis_7j)
        .fetch_one(pool)
        .await?;
        let actuel = ts.is_some_and(|ts| maintenant - ts <= ETAT_SALLE_MAX_S);
        sortie.push(json!({
            "id": salle_id,
            "nom": ligne.try_get::<String, _>("nom")?,
            "pilote": ligne.try_get::<i64, _>("pilote").unwrap_or(0) == 1,
            "seuil_min": seuil_min,
            "seuil_max": seuil_max,
            "maj": ts.map(heure_locale),
            "actuel": actuel,
            "t": ligne.try_get::<Option<f64>, _>("t").ok().flatten(),
            "h": ligne.try_get::<Option<f64>, _>("h").ok().flatten(),
            "consigne": ligne.try_get::<Option<f64>, _>("consigne").ok().flatten(),
            "ventil": ligne.try_get::<Option<f64>, _>("ventil").ok().flatten(),
            "mode": ligne.try_get::<Option<String>, _>("mode").ok().flatten(),
            "sonde_ok": ligne.try_get::<i64, _>("sonde_ok").unwrap_or(1) == 1,
            "message": ligne.try_get::<Option<String>, _>("message").ok().flatten(),
            "bande": ligne.try_get::<Option<String>, _>("bande").ok().flatten(),
            "jour_bande": ligne.try_get::<Option<i64>, _>("jour_bande").ok().flatten(),
            "t24": t24.json(),
            "t7": t7.json(),
            "h24": h24.json(),
            "consigne24": c24.moyenne,
            "ecart24": ecart,
            "hors_seuils_24": part_hors_seuils(pool, salle_id, depuis_24h, seuil_min, seuil_max).await?,
            "hors_seuils_7j": part_hors_seuils(pool, salle_id, depuis_7j, seuil_min, seuil_max).await?,
            "alertes_7j": alertes,
        }));
    }
    Ok(sortie)
}

async fn liaison_regulation(pool: &SqlitePool, maintenant: i64) -> AppResult<Value> {
    let ligne = sqlx::query("SELECT ts,version,cycles,erreurs FROM ambiance_heartbeat WHERE id=1")
        .fetch_optional(pool)
        .await?;
    let Some(ligne) = ligne else {
        return Ok(json!({"connu": false}));
    };
    let ts: i64 = ligne.try_get("ts")?;
    let simulation: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ambiance_etat e JOIN ambiance_salle a ON a.salle_id=e.salle_id WHERE a.actif=1 AND e.mode='simulation'",
    )
    .fetch_one(pool)
    .await?;
    Ok(json!({
        "connu": true,
        "age_s": maintenant - ts,
        "actif": maintenant - ts <= BATTEMENT_MAX_S,
        "maj": heure_locale(ts),
        "version": ligne.try_get::<Option<String>, _>("version").ok().flatten(),
        "cycles": ligne.try_get::<i64, _>("cycles").unwrap_or(0),
        "erreurs": ligne.try_get::<i64, _>("erreurs").unwrap_or(0),
        "simulation": simulation > 0,
        "version_web": env!("CARGO_PKG_VERSION"),
    }))
}

struct Action {
    // 0 = en retard / critique, 1 = aujourd'hui, 2 = dans les 48 h.
    priorite: u8,
    date: String,
    categorie: &'static str,
    texte: String,
    lien: String,
}

fn priorite_date(date: NaiveDate, aujourd_hui: NaiveDate) -> u8 {
    if date < aujourd_hui {
        0
    } else if date == aujourd_hui {
        1
    } else {
        2
    }
}

async fn actions_48h(
    pool: &SqlitePool,
    session: &SessionData,
    salles: &[Value],
    liaison: &Value,
) -> AppResult<Vec<Value>> {
    let pages = crate::auth::effective_pages(session);
    let voit = |page: &str| pages.iter().any(|p| p == page);
    let aujourd_hui = Local::now().date_naive();
    let limite = aujourd_hui + Duration::days(HORIZON_JOURS);
    let limite_iso = limite.format("%Y-%m-%d").to_string();
    let mut actions: Vec<Action> = Vec::new();

    // Ambiance : ce qui demande une vérification sur place.
    if voit("ambiance") {
        if liaison["connu"].as_bool() == Some(true) && liaison["actif"].as_bool() != Some(true) {
            actions.push(Action {
                priorite: 0,
                date: aujourd_hui.to_string(),
                categorie: "Ventilation",
                texte: format!(
                    "La régulation (eo-regul) ne répond plus depuis le {} : vérifier le service",
                    liaison["maj"].as_str().unwrap_or("—")
                ),
                lien: "/ambiance".into(),
            });
        }
        let ouvertes = sqlx::query(
            "SELECT a.debut,a.message,a.gravite,s.nom FROM ambiance_alerte a LEFT JOIN salle s ON s.id=a.salle_id \
             WHERE a.fin IS NULL AND a.acquittee=0 ORDER BY a.debut",
        )
        .fetch_all(pool)
        .await?;
        for alerte in ouvertes {
            let salle: Option<String> = alerte.try_get::<Option<String>, _>("nom").ok().flatten();
            let message: String = alerte.try_get("message").unwrap_or_default();
            let gravite: String = alerte.try_get("gravite").unwrap_or_default();
            actions.push(Action {
                priorite: if gravite == "critique" { 0 } else { 1 },
                date: alerte.try_get::<String, _>("debut").unwrap_or_default(),
                categorie: "Ventilation",
                texte: format!("{} : {message}", salle.as_deref().unwrap_or("Ambiance")),
                lien: "/ambiance".into(),
            });
        }
        for salle in salles {
            let nom = salle["nom"].as_str().unwrap_or("Salle");
            if salle["actuel"].as_bool() != Some(true) {
                actions.push(Action {
                    priorite: 0,
                    date: aujourd_hui.to_string(),
                    categorie: "Ventilation",
                    texte: format!(
                        "{nom} : aucune mesure récente (dernière le {}) — vérifier sonde et automate",
                        salle["maj"].as_str().unwrap_or("—")
                    ),
                    lien: format!("/ambiance/salle/{}/historique", salle["id"]),
                });
            } else if salle["sonde_ok"].as_bool() == Some(false) {
                actions.push(Action {
                    priorite: 0,
                    date: aujourd_hui.to_string(),
                    categorie: "Ventilation",
                    texte: format!("{nom} : sonde en défaut, salle en repli de sécurité"),
                    lien: format!("/ambiance/salle/{}/historique", salle["id"]),
                });
            }
            if let Some(part) = salle["hors_seuils_24"].as_f64() {
                if part >= 10.0 {
                    actions.push(Action {
                        priorite: 1,
                        date: aujourd_hui.to_string(),
                        categorie: "Ventilation",
                        texte: format!(
                            "{nom} : {part:.0} % des relevés des dernières 24 h hors des seuils d'alerte — contrôler le réglage de la salle"
                        ),
                        lien: format!("/ambiance/salle/{}/historique", salle["id"]),
                    });
                }
            }
        }
    }

    // Étapes des bandes (IA, échographie, mise-bas, sevrage…).
    if voit("bandes") || voit("planning") {
        let bandes = sqlx::query_as::<_, Bande>(BAND_SELECT_ACTIVE)
            .fetch_all(pool)
            .await?;
        let calendrier = load_band_schedule(pool).await?;
        for bande in bandes {
            for etape in key_dates(bande.date_mb.as_deref(), calendrier) {
                let Some(date) = etape["date"].as_str().and_then(parse_stored_date) else {
                    continue;
                };
                if date < aujourd_hui || date > limite {
                    continue;
                }
                actions.push(Action {
                    priorite: priorite_date(date, aujourd_hui),
                    date: date.to_string(),
                    categorie: "Bandes",
                    texte: format!(
                        "Bande {} : {}",
                        bande.code,
                        etape["nom"].as_str().unwrap_or("étape")
                    ),
                    lien: if etape["nom"].as_str() == Some("Sevrage") {
                        format!("/bande/{}/sevrage", bande.id)
                    } else {
                        format!("/bande/{}", bande.id)
                    },
                });
            }
        }
    }

    // Protocoles sanitaires de bande avec rappel, non réalisés.
    if voit("sanitaire") {
        let rappels = sqlx::query(
            "SELECT ap.libelle,ap.produit,b.id AS bande_id,b.code,date(b.date_mb,printf('%+d day',ap.jour)) AS echeance \
             FROM acteprotocole ap JOIN bande b ON b.active=1 \
             WHERE ap.actif=1 AND ap.rappel=1 AND ap.reference='mise_bas' AND b.date_mb IS NOT NULL \
             AND NOT EXISTS(SELECT 1 FROM acterealise ar WHERE ar.acte_id=ap.id AND ar.bande_id=b.id) \
             AND date(b.date_mb,printf('%+d day',ap.jour))<=?",
        )
        .bind(&limite_iso)
        .fetch_all(pool)
        .await?;
        for rappel in rappels {
            let echeance: String = rappel.try_get("echeance").unwrap_or_default();
            let Some(date) = parse_stored_date(&echeance) else {
                continue;
            };
            let produit: Option<String> = rappel
                .try_get::<Option<String>, _>("produit")
                .ok()
                .flatten();
            actions.push(Action {
                priorite: priorite_date(date, aujourd_hui),
                date: echeance,
                categorie: "Sanitaire",
                texte: format!(
                    "Bande {} : {}{}",
                    rappel.try_get::<String, _>("code").unwrap_or_default(),
                    rappel.try_get::<String, _>("libelle").unwrap_or_default(),
                    produit.map(|p| format!(" ({p})")).unwrap_or_default()
                ),
                lien: "/sanitaire".into(),
            });
        }
        // Soins de portée, regroupés par bande et par soin.
        let soins = sqlx::query(
            "SELECT sp.date_prevue,a.libelle,b.id AS bande_id,COALESCE(b.code,'sans bande') AS code,COUNT(*) AS n \
             FROM soinportee sp JOIN acteprotocole a ON a.id=sp.protocole_id LEFT JOIN bande b ON b.id=sp.bande_id \
             WHERE sp.date_realisee IS NULL AND date(sp.date_prevue)<=? AND date(sp.date_prevue)>=date('now',?) \
             GROUP BY sp.date_prevue,a.libelle,sp.bande_id ORDER BY sp.date_prevue",
        )
        .bind(&limite_iso)
        .bind(format!("-{RETARD_SOINS_MAX_JOURS} day"))
        .fetch_all(pool)
        .await?;
        for soin in soins {
            let prevue: String = soin.try_get("date_prevue").unwrap_or_default();
            let Some(date) = parse_stored_date(&prevue) else {
                continue;
            };
            let n: i64 = soin.try_get("n").unwrap_or(0);
            let bande_id: Option<i64> = soin.try_get::<Option<i64>, _>("bande_id").ok().flatten();
            actions.push(Action {
                priorite: priorite_date(date, aujourd_hui),
                date: prevue,
                categorie: "Soins porcelets",
                texte: format!(
                    "Bande {} : {} ({n} portée{})",
                    soin.try_get::<String, _>("code").unwrap_or_default(),
                    soin.try_get::<String, _>("libelle").unwrap_or_default(),
                    if n > 1 { "s" } else { "" }
                ),
                lien: bande_id
                    .map(|id| format!("/maternite/bande/{id}"))
                    .unwrap_or_else(|| "/sanitaire".into()),
            });
        }
    }

    // Tâches et entretiens.
    if voit("entretien") {
        let taches = sqlx::query(
            "SELECT titre,echeance,bande_code,salle FROM tache WHERE fait=0 AND echeance IS NOT NULL AND date(echeance)<=?",
        )
        .bind(&limite_iso)
        .fetch_all(pool)
        .await?;
        for tache in taches {
            let echeance: String = tache.try_get("echeance").unwrap_or_default();
            let Some(date) = parse_stored_date(&echeance) else {
                continue;
            };
            let precision: Vec<String> = [
                tache
                    .try_get::<Option<String>, _>("bande_code")
                    .ok()
                    .flatten(),
                tache.try_get::<Option<String>, _>("salle").ok().flatten(),
            ]
            .into_iter()
            .flatten()
            .filter(|texte| !texte.trim().is_empty())
            .collect();
            actions.push(Action {
                priorite: priorite_date(date, aujourd_hui),
                date: echeance,
                categorie: "Tâches",
                texte: format!(
                    "{}{}",
                    tache.try_get::<String, _>("titre").unwrap_or_default(),
                    if precision.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", precision.join(" · "))
                    }
                ),
                lien: "/taches".into(),
            });
        }
        let entretiens = sqlx::query(
            "SELECT nom,site,date(derniere_date,'+'||frequence_jours||' days') AS prochaine FROM entretien \
             WHERE derniere_date IS NOT NULL AND date(derniere_date,'+'||frequence_jours||' days')<=?",
        )
        .bind(&limite_iso)
        .fetch_all(pool)
        .await?;
        for entretien in entretiens {
            let prochaine: String = entretien.try_get("prochaine").unwrap_or_default();
            let Some(date) = parse_stored_date(&prochaine) else {
                continue;
            };
            let site: Option<String> = entretien
                .try_get::<Option<String>, _>("site")
                .ok()
                .flatten();
            actions.push(Action {
                priorite: priorite_date(date, aujourd_hui),
                date: prochaine,
                categorie: "Entretien",
                texte: format!(
                    "{}{}",
                    entretien.try_get::<String, _>("nom").unwrap_or_default(),
                    site.filter(|s| !s.trim().is_empty())
                        .map(|s| format!(" · {s}"))
                        .unwrap_or_default()
                ),
                lien: "/entretien".into(),
            });
        }
    }

    actions.sort_by(|a, b| {
        a.priorite
            .cmp(&b.priorite)
            .then_with(|| a.date.cmp(&b.date))
            .then_with(|| a.categorie.cmp(b.categorie))
    });
    Ok(actions
        .into_iter()
        .map(|action| {
            json!({
                "priorite": action.priorite,
                "date": action.date,
                "categorie": action.categorie,
                "texte": action.texte,
                "lien": action.lien,
            })
        })
        .collect())
}

/// Requête à paramètres texte, rendue en JSON comme `generic_rows`.
async fn lignes(pool: &SqlitePool, sql: &str, parametres: &[&str]) -> AppResult<Vec<Value>> {
    let mut requete = sqlx::query(sql);
    for parametre in parametres {
        requete = requete.bind(*parametre);
    }
    rows_to_json(requete.fetch_all(pool).await?)
}

async fn une_ligne(pool: &SqlitePool, sql: &str, parametres: &[&str]) -> AppResult<Value> {
    Ok(lignes(pool, sql, parametres)
        .await?
        .into_iter()
        .next()
        .unwrap_or_else(|| json!({})))
}

/// Bandes actives : étape en cours, prochaine étape, truies et porcs.
async fn bandes_actives(pool: &SqlitePool) -> AppResult<(Vec<Value>, i64)> {
    let bandes = sqlx::query_as::<_, Bande>(BAND_SELECT_ACTIVE)
        .fetch_all(pool)
        .await?;
    let calendrier = load_band_schedule(pool).await?;
    let aujourd_hui = Local::now().date_naive();
    let mut sortie = Vec::with_capacity(bandes.len());
    let mut total_porcs = 0;
    for bande in bandes {
        let etapes = key_dates(bande.date_mb.as_deref(), calendrier);
        let actuelle = etapes
            .iter()
            .find(|e| e["actuelle"].as_bool() == Some(true))
            .and_then(|e| e["nom"].as_str())
            .map(str::to_string);
        let prochaine = etapes
            .iter()
            .find(|e| e["etat"].as_str() == Some("À venir"));
        let delai = prochaine
            .and_then(|e| e["date"].as_str())
            .and_then(parse_stored_date)
            .map(|date| (date - aujourd_hui).num_days());
        let truies: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE reformee=0 AND bande_code=?")
                .bind(&bande.code)
                .fetch_one(pool)
                .await?;
        let porcs = remaining_band_pigs(pool, bande.id, &bande.code)
            .await
            .unwrap_or(0)
            .max(0);
        total_porcs += porcs;
        sortie.push(json!({
            "id": bande.id,
            "code": bande.code,
            "marquage": bande.num_officiel,
            "site": bande.site,
            "date_mb": bande.date_mb,
            "etape": actuelle,
            "prochaine": prochaine.and_then(|e| e["nom"].as_str()),
            "prochaine_date": prochaine.and_then(|e| e["date"].as_str()),
            "delai": delai,
            "truies": truies,
            "porcs": porcs,
        }));
    }
    Ok((sortie, total_porcs))
}

/// Consommation d'un compteur sur la période : écart d'index entre le dernier
/// relevé avant la période et le dernier relevé de la période. Un
/// remplacement de compteur dans l'intervalle rend l'écart inexploitable : on
/// le signale plutôt que d'afficher un chiffre faux.
async fn energie(pool: &SqlitePool, debut: &str, fin: &str) -> AppResult<Vec<Value>> {
    let compteurs = lignes(
        pool,
        "SELECT id,nom,type,unite FROM compteur_energie WHERE actif=1 ORDER BY type,nom",
        &[],
    )
    .await?;
    let mut sortie = Vec::new();
    for compteur in compteurs {
        let id = compteur["id"].as_i64().unwrap_or_default().to_string();
        let bornes = une_ligne(
            pool,
            "SELECT (SELECT valeur_index FROM releve_compteur WHERE compteur_id=?1 AND date_releve<?2 ORDER BY date_releve DESC,id DESC LIMIT 1) AS avant,\
             (SELECT date_releve FROM releve_compteur WHERE compteur_id=?1 AND date_releve<?2 ORDER BY date_releve DESC,id DESC LIMIT 1) AS date_avant,\
             (SELECT valeur_index FROM releve_compteur WHERE compteur_id=?1 AND date_releve<=?3 ORDER BY date_releve DESC,id DESC LIMIT 1) AS apres,\
             (SELECT date_releve FROM releve_compteur WHERE compteur_id=?1 AND date_releve<=?3 ORDER BY date_releve DESC,id DESC LIMIT 1) AS date_apres,\
             (SELECT COUNT(*) FROM releve_compteur WHERE compteur_id=?1 AND date_releve>=?2 AND date_releve<=?3 AND COALESCE(remplacement_compteur,0)=1) AS remplacements",
            &[&id, debut, fin],
        )
        .await?;
        let consommation = match (bornes["avant"].as_f64(), bornes["apres"].as_f64()) {
            (Some(avant), Some(apres))
                if bornes["remplacements"].as_i64() == Some(0)
                    && bornes["date_apres"] != bornes["date_avant"]
                    && apres >= avant =>
            {
                Some(apres - avant)
            }
            _ => None,
        };
        let jours = match (
            bornes["date_avant"].as_str().and_then(parse_stored_date),
            bornes["date_apres"].as_str().and_then(parse_stored_date),
        ) {
            (Some(du), Some(au)) => Some((au - du).num_days()).filter(|j| *j > 0),
            _ => None,
        };
        let par_jour = match (consommation, jours) {
            (Some(total), Some(jours)) => Some(total / jours as f64),
            _ => None,
        };
        sortie.push(json!({
            "jours": jours,
            "par_jour": par_jour,
            "nom": compteur["nom"],
            "type": compteur["type"],
            "unite": compteur["unite"],
            "consommation": consommation,
            "du": bornes["date_avant"],
            "au": bornes["date_apres"],
            "remplacement": bornes["remplacements"].as_i64().unwrap_or(0) > 0,
        }));
    }
    Ok(sortie)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let pages = crate::auth::effective_pages(&session);
    let voit = |page: &str| pages.iter().any(|p| p == page);
    let jours: i64 = match query.get("jours").map(String::as_str) {
        Some("14") => 14,
        Some("30") => 30,
        _ => 7,
    };
    let aujourd_hui = Local::now().date_naive();
    let debut = (aujourd_hui - Duration::days(jours - 1)).to_string();
    let fin = aujourd_hui.to_string();
    let periode: [&str; 2] = [&debut, &fin];
    let maintenant = maintenant_ts();

    let voit_ambiance = voit("ambiance");
    let (salles, liaison) = if voit_ambiance {
        (
            salles(pool, maintenant).await?,
            liaison_regulation(pool, maintenant).await?,
        )
    } else {
        (Vec::new(), json!({"connu": false}))
    };
    let actions = actions_48h(pool, &session, &salles, &liaison).await?;

    let mut ctx = context(&session);
    if voit("planning") || voit("bandes") {
        ctx.insert(
            "a_anticiper".into(),
            Value::Array(super::conduite::alertes(pool).await.unwrap_or_default()),
        );
    }

    // Chiffres clés de la période.
    let mises_bas = une_ligne(pool, "SELECT SUM(CASE WHEN COALESCE(nes_totaux,0)>0 THEN 1 ELSE 0 END) AS n,SUM(CASE WHEN COALESCE(nes_totaux,0)=0 THEN 1 ELSE 0 END) AS a_completer,COALESCE(SUM(nes_totaux),0) AS nt,COALESCE(SUM(nes_vifs),0) AS nv,COALESCE(SUM(mort_nes),0) AS mn,ROUND(AVG(NULLIF(nes_totaux,0)),1) AS nt_portee,ROUND(AVG(CASE WHEN COALESCE(nes_totaux,0)>0 THEN nes_vifs END),1) AS nv_portee FROM evenement WHERE type='mise_bas' AND date BETWEEN ?1 AND ?2", &periode).await?;
    let sevrages = une_ligne(pool, "SELECT COUNT(*) AS n,COALESCE(SUM(nb_sevres),0) AS sevres,ROUND(AVG(nb_sevres),1) AS par_portee,ROUND(AVG(NULLIF(poids_moyen,0)),1) AS poids FROM evenement WHERE type='sevrage' AND date BETWEEN ?1 AND ?2", &periode).await?;
    let repro = une_ligne(pool, "SELECT COUNT(DISTINCT CASE WHEN type='ia' THEN truie_id END) AS ia,\
         SUM(CASE WHEN type IN ('echo','echographie') AND resultat='pleine' THEN 1 ELSE 0 END) AS pleines,\
         SUM(CASE WHEN type IN ('echo','echographie') AND resultat='vide' THEN 1 ELSE 0 END) AS vides,\
         SUM(CASE WHEN type='chaleur' THEN 1 ELSE 0 END) AS chaleurs \
         FROM evenement WHERE date BETWEEN ?1 AND ?2", &periode).await?;
    let pertes_porcelets: i64 = sqlx::query_scalar(
        "SELECT CAST(COALESCE(SUM(nb),0) AS INTEGER) FROM perteporcelet WHERE date BETWEEN ? AND ?",
    )
    .bind(&debut)
    .bind(&fin)
    .fetch_one(pool)
    .await?;
    let morts_porcs: i64 = sqlx::query_scalar("SELECT CAST(COALESCE(SUM(nombre),0) AS INTEGER) FROM declarationmort WHERE date BETWEEN ? AND ? AND (stade IS NULL OR stade NOT LIKE 'mater%') AND NOT EXISTS(SELECT 1 FROM perteporcelet p JOIN bande b ON b.id=p.bande_id WHERE b.code=declarationmort.bande_code AND p.date=declarationmort.date AND lower(COALESCE(p.cause,''))=lower(COALESCE(declarationmort.cause,'')))").bind(&debut).bind(&fin).fetch_one(pool).await?;
    let truies_presentes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE reformee=0")
        .fetch_one(pool)
        .await?;
    let (bandes, porcs) = if voit("bandes") {
        bandes_actives(pool).await?
    } else {
        (Vec::new(), 0)
    };
    let nv = mises_bas["nv"].as_i64().unwrap_or(0);
    let taux_pertes = (nv > 0).then(|| 100.0 * pertes_porcelets as f64 / nv as f64);
    ctx.insert("synthese".into(), json!({
        "truies": truies_presentes,
        "bandes": bandes.len(),
        "porcs": porcs,
        "mises_bas": mises_bas,
        "sevrages": sevrages,
        "repro": repro,
        "pertes_porcelets": pertes_porcelets,
        "taux_pertes": taux_pertes,
        "morts_porcs": morts_porcs,
        "actions_urgentes": actions.iter().filter(|a| a["priorite"].as_u64() == Some(0)).count(),
    }));
    ctx.insert("bandes".into(), Value::Array(bandes));

    if voit("bandes") || voit("truies") {
        ctx.insert("liste_mises_bas".into(), Value::Array(lignes(pool, "SELECT e.date,t.num_travail AS truie,b.code AS bande,e.nes_totaux AS nt,e.nes_vifs AS nv,e.mort_nes AS mn,e.momifies AS mo FROM evenement e LEFT JOIN truie t ON t.id=e.truie_id LEFT JOIN bande b ON b.id=e.bande_id WHERE e.type='mise_bas' AND e.date BETWEEN ?1 AND ?2 ORDER BY e.date DESC,e.id DESC", &periode).await?));
        ctx.insert("liste_sevrages".into(), Value::Array(lignes(pool, "SELECT b.code AS bande,COUNT(*) AS portees,COALESCE(SUM(e.nb_sevres),0) AS sevres,ROUND(AVG(NULLIF(e.poids_moyen,0)),1) AS poids,MIN(e.date) AS du,MAX(e.date) AS au FROM evenement e LEFT JOIN bande b ON b.id=e.bande_id WHERE e.type='sevrage' AND e.date BETWEEN ?1 AND ?2 GROUP BY e.bande_id ORDER BY au DESC", &periode).await?));
        ctx.insert("pertes_causes".into(), Value::Array(lignes(pool, "SELECT COALESCE(NULLIF(trim(cause),''),'Non précisée') AS cause,CAST(SUM(nb) AS INTEGER) AS n FROM perteporcelet WHERE date BETWEEN ?1 AND ?2 GROUP BY 1 ORDER BY n DESC", &periode).await?));
        ctx.insert("mortalite".into(), Value::Array(lignes(pool, "SELECT COALESCE(bande_code,'—') AS bande,COALESCE(NULLIF(trim(cause),''),'Non précisée') AS cause,CAST(SUM(nombre) AS INTEGER) AS n FROM declarationmort WHERE date BETWEEN ?1 AND ?2 AND (stade IS NULL OR stade NOT LIKE 'mater%') AND NOT EXISTS(SELECT 1 FROM perteporcelet p JOIN bande b ON b.id=p.bande_id WHERE b.code=declarationmort.bande_code AND p.date=declarationmort.date AND lower(COALESCE(p.cause,''))=lower(COALESCE(declarationmort.cause,''))) GROUP BY 1,2 ORDER BY n DESC", &periode).await?));
        ctx.insert("reformes".into(), Value::Array(lignes(pool, "SELECT num_travail AS truie,date_reforme AS date,COALESCE(motif_sortie,'—') AS motif,rang FROM truie WHERE date_reforme BETWEEN ?1 AND ?2 ORDER BY date_reforme DESC", &periode).await?));
        ctx.insert("ia_bandes".into(), Value::Array(lignes(pool, "SELECT COALESCE(b.code,'Sans bande') AS bande,COUNT(DISTINCT e.truie_id) AS truies,MIN(e.date) AS du,MAX(e.date) AS au FROM evenement e LEFT JOIN bande b ON b.id=e.bande_id WHERE e.type='ia' AND e.date BETWEEN ?1 AND ?2 GROUP BY e.bande_id ORDER BY au DESC", &periode).await?));
        ctx.insert("vides".into(), Value::Array(lignes(pool, "SELECT e.date,t.num_travail AS truie,b.code AS bande FROM evenement e LEFT JOIN truie t ON t.id=e.truie_id LEFT JOIN bande b ON b.id=e.bande_id WHERE e.type IN ('echo','echographie') AND e.resultat='vide' AND e.date BETWEEN ?1 AND ?2 ORDER BY e.date DESC", &periode).await?));
    }
    if voit("sanitaire") {
        ctx.insert("traitements".into(), Value::Array(lignes(pool, "SELECT e.date,e.date_fin,t.num_travail AS truie,b.code AS bande,e.produit,e.motif FROM evenement e LEFT JOIN truie t ON t.id=e.truie_id LEFT JOIN bande b ON b.id=e.bande_id WHERE e.type='traitement' AND e.date BETWEEN ?1 AND ?2 ORDER BY e.date DESC", &periode).await?));
        ctx.insert("delais_attente".into(), Value::Array(lignes(pool, "SELECT t.num_travail AS truie,e.produit,date(COALESCE(e.date_fin,e.date),printf('+%d day',CAST(e.delai_attente AS INTEGER))) AS fin FROM evenement e JOIN truie t ON t.id=e.truie_id WHERE e.type='traitement' AND CAST(COALESCE(e.delai_attente,0) AS INTEGER)>0 AND date(COALESCE(e.date_fin,e.date),printf('+%d day',CAST(e.delai_attente AS INTEGER)))>=?1 ORDER BY fin", &[&fin]).await?));
    }
    if voit("stock") {
        ctx.insert("pharmacie_basse".into(), Value::Array(lignes(pool, "SELECT produit,stock_actuel,unite,seuil_alerte FROM produitpharmacie WHERE seuil_alerte IS NOT NULL AND stock_actuel<=seuil_alerte ORDER BY produit", &[]).await?));
        ctx.insert("silos".into(), Value::Array(lignes(pool, "SELECT s.nom,s.capacite_tonnes AS capacite,r.niveau_tonnes AS niveau,r.date FROM silo_aliment s LEFT JOIN releve_silo r ON r.id=(SELECT id FROM releve_silo WHERE silo_id=s.id ORDER BY date DESC,id DESC LIMIT 1) WHERE s.actif=1 ORDER BY s.nom", &[]).await?));
    }
    if voit("economique") {
        ctx.insert("ventes".into(), une_ligne(pool, "SELECT COUNT(*) AS apports,CAST(COALESCE(SUM(nb_porcs),0) AS INTEGER) AS porcs,ROUND(SUM(poids_total)/NULLIF(SUM(nb_porcs),0),1) AS poids,ROUND(SUM(montant_net),2) AS montant,ROUND(SUM(prix_moyen*nb_porcs)/NULLIF(SUM(CASE WHEN prix_moyen IS NOT NULL THEN nb_porcs END),0),3) AS prix FROM venteapport WHERE date BETWEEN ?1 AND ?2", &periode).await?);
    }
    if voit("energie") {
        ctx.insert(
            "energie".into(),
            Value::Array(energie(pool, &debut, &fin).await?),
        );
    }

    let nom_elevage: Option<String> =
        sqlx::query_scalar("SELECT valeur FROM parametre WHERE cle='nom_elevage'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    ctx.insert("jours".into(), json!(jours));
    ctx.insert("debut".into(), json!(debut));
    ctx.insert("fin".into(), json!(fin));
    ctx.insert("voit_ambiance".into(), json!(voit_ambiance));
    ctx.insert("salles".into(), Value::Array(salles));
    ctx.insert("liaison".into(), liaison);
    ctx.insert("actions".into(), Value::Array(actions));
    ctx.insert("nom_elevage".into(), json!(nom_elevage));
    ctx.insert(
        "genere_le".into(),
        json!(Local::now().format("%d/%m/%Y à %Hh%M").to_string()),
    );
    ctx.insert("aujourd_hui".into(), json!(fin));
    ctx.insert(
        "limite".into(),
        json!((aujourd_hui + Duration::days(HORIZON_JOURS)).to_string()),
    );
    render(&state, "rapport.html", Value::Object(ctx))
}

/// Ancienne adresse du bouton « Rapport » (bilan hebdomadaire PDF) : renvoie
/// vers la page imprimable pour ne pas casser les favoris.
pub(super) async fn ancien_pdf() -> Redirect {
    Redirect::to("/rapport")
}

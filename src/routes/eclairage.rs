//! Programmes d'éclairage : la page de réglage (/ambiance/eclairage).
//!
//! Comme pour la ventilation, l'application n'écrit ici que la
//! configuration. C'est le démon `eo-regul` qui l'applique, avec la même
//! fonction de décision (`eclairage_core`) — la page s'en sert seulement pour
//! dire en clair ce que chaque programme fait et fera.

use super::*;
use chrono::TimeZone;
use crate::eclairage_core::{
    dans_horaire, heure_depuis_minutes, jours_depuis_texte, jours_en_texte, minutes_depuis_heure,
    prochain_changement, Plage,
};

const JOURS_COURTS: [&str; 7] = ["L", "M", "M", "J", "V", "S", "D"];
const JOURS_LONGS: [&str; 7] = [
    "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
];
/// Plages lues au plus dans un formulaire : bien au-delà de l'usage réel.
const PLAGES_MAX: usize = 24;

fn maintenant() -> i64 {
    chrono::Utc::now().timestamp()
}

fn jour_et_minute() -> (usize, u32) {
    use chrono::{Datelike, Timelike};
    let local = Local::now();
    (
        local.weekday().num_days_from_monday() as usize,
        local.hour() * 60 + local.minute(),
    )
}

/// « du lundi au vendredi », « le week-end », « lundi, mercredi et vendredi ».
fn jours_lisibles(jours: &[bool; 7]) -> String {
    let nombre = jours.iter().filter(|jour| **jour).count();
    if nombre == 7 {
        return "tous les jours".into();
    }
    if nombre == 0 {
        return "aucun jour".into();
    }
    if *jours == [true, true, true, true, true, false, false] {
        return "du lundi au vendredi".into();
    }
    if *jours == [false, false, false, false, false, true, true] {
        return "le week-end".into();
    }
    if *jours == [true, true, true, true, true, true, false] {
        return "du lundi au samedi".into();
    }
    let noms: Vec<&str> = (0..7).filter(|j| jours[*j]).map(|j| JOURS_LONGS[j]).collect();
    match noms.split_last() {
        Some((dernier, [])) => format!("le {dernier}"),
        Some((dernier, debut)) => format!("{} et {dernier}", debut.join(", ")),
        None => String::new(),
    }
}

fn plage_lisible(plage: &Plage) -> String {
    if plage.debut == plage.fin {
        return format!("{} toute la journée", jours_lisibles(&plage.jours));
    }
    format!(
        "{} de {} à {}{}",
        jours_lisibles(&plage.jours),
        heure_depuis_minutes(plage.debut).replace(':', "h"),
        heure_depuis_minutes(plage.fin).replace(':', "h"),
        if plage.fin < plage.debut { " le lendemain" } else { "" }
    )
}

fn nombre_lisible(valeur: f64) -> String {
    if (valeur - valeur.round()).abs() < 1e-9 {
        format!("{}", valeur.round() as i64)
    } else {
        format!("{valeur}").replace('.', ",")
    }
}

fn duree_minutes(minutes: i64) -> String {
    if minutes >= 60 && minutes % 60 == 0 {
        format!("{} h", minutes / 60)
    } else if minutes > 60 {
        format!("{} h {:02}", minutes / 60, minutes % 60)
    } else {
        format!("{minutes} min")
    }
}

/// Programmes lus en base, prêts pour la page : réglages, résumé en clair,
/// frise de la semaine et état tenu par le démon.
async fn charger_programmes(pool: &SqlitePool, seul: Option<i64>) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT p.id, p.nom, p.actif, p.essai, p.lux_point_id, p.lux_allumer_sous, \
                p.lux_eteindre_au_dessus, p.lux_temporisation_min, p.lux_si_muet, \
                p.minuterie_min, p.minuterie_manuel, p.valeur_allume, p.valeur_eteint, \
                p.forcage, p.forcage_fin, p.note, l.nom AS lux_nom \
         FROM eclairage_programme p LEFT JOIN ipx_point l ON l.id=p.lux_point_id \
         WHERE (? IS NULL OR p.id=?) ORDER BY p.nom COLLATE NOCASE, p.id",
    )
    .bind(seul)
    .bind(seul)
    .fetch_all(pool)
    .await?;

    let (jour, minute) = jour_et_minute();
    let ts = maintenant();
    let mut sortie = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        let id: i64 = ligne.try_get("id")?;
        let plages: Vec<Plage> = sqlx::query_as::<_, (String, i64, i64)>(
            "SELECT jours,debut,fin FROM eclairage_plage WHERE programme_id=? ORDER BY id",
        )
        .bind(id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|(jours, debut, fin)| Plage {
            jours: jours_depuis_texte(&jours),
            debut: debut.clamp(0, 1439) as u32,
            fin: fin.clamp(0, 1439) as u32,
        })
        .collect();

        let sorties = sqlx::query(
            "SELECT p.id, p.nom, s2.nom AS salle, e.allume, e.allume_depuis, e.demande, \
                    e.raison, e.erreur, e.maj, e.lux, e.force \
             FROM eclairage_sortie s JOIN ipx_point p ON p.id=s.point_id \
             LEFT JOIN salle s2 ON s2.id=p.salle_id \
             LEFT JOIN eclairage_etat e ON e.point_id=p.id AND e.programme_id=s.programme_id \
             WHERE s.programme_id=? ORDER BY p.nom",
        )
        .bind(id)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|s| {
            let allume = s.try_get::<Option<i64>, _>("allume").ok().flatten();
            let depuis = s.try_get::<Option<i64>, _>("allume_depuis").ok().flatten();
            let maj = s.try_get::<Option<i64>, _>("maj").ok().flatten();
            json!({
                "id": s.try_get::<i64, _>("id").unwrap_or(0),
                "nom": s.try_get::<String, _>("nom").unwrap_or_default(),
                "salle": s.try_get::<Option<String>, _>("salle").ok().flatten(),
                "allume": allume.map(|v| v != 0),
                "allume_depuis_min": depuis.filter(|_| allume == Some(1)).map(|d| (ts - d).max(0) / 60),
                "raison": s.try_get::<Option<String>, _>("raison").ok().flatten(),
                "erreur": s.try_get::<Option<String>, _>("erreur").ok().flatten(),
                "lux": s.try_get::<Option<f64>, _>("lux").ok().flatten().map(|l| l.round() as i64),
                "maj_s": maj.map(|m| (ts - m).max(0)),
            })
        })
        .collect::<Vec<_>>();

        let lux_point: Option<i64> = ligne.try_get("lux_point_id").ok().flatten();
        let allumer_sous: Option<f64> = ligne.try_get("lux_allumer_sous").ok().flatten();
        let eteindre: Option<f64> = ligne.try_get("lux_eteindre_au_dessus").ok().flatten();
        let lux_actif = lux_point.is_some() && allumer_sous.is_some();
        let minuterie: Option<i64> = ligne
            .try_get::<Option<i64>, _>("minuterie_min")
            .ok()
            .flatten()
            .filter(|m| *m > 0);
        let minuterie_manuel = ligne.try_get::<i64, _>("minuterie_manuel").unwrap_or(1) != 0;
        let essai = ligne.try_get::<i64, _>("essai").unwrap_or(1) != 0;
        let actif = ligne.try_get::<i64, _>("actif").unwrap_or(1) != 0;
        let forcage = ligne.try_get::<Option<i64>, _>("forcage").ok().flatten();
        let forcage_fin = ligne.try_get::<Option<i64>, _>("forcage_fin").ok().flatten();
        let forcage = match (forcage, forcage_fin) {
            (Some(f), Some(fin)) if fin > ts => Some(json!({
                "allume": f != 0,
                "reste_min": ((fin - ts) + 59) / 60,
                "fin": Local
                    .timestamp_opt(fin, 0)
                    .single()
                    .map(|d| d.format("%H:%M").to_string()),
            })),
            _ => None,
        };

        // Phrase de résumé : ce que le programme fait, en français courant.
        let mut phrases: Vec<String> = Vec::new();
        if !plages.is_empty() {
            let liste: Vec<String> = plages.iter().map(plage_lisible).collect();
            let mut phrase = format!("Allume {}", liste.join(" ; "));
            if lux_actif {
                phrase.push_str(&format!(
                    ", seulement s'il fait moins de {} lux",
                    nombre_lisible(allumer_sous.unwrap_or(0.0))
                ));
            }
            phrases.push(format!("{phrase}."));
        } else if lux_actif {
            phrases.push(format!(
                "Allume dès qu'il fait moins de {} lux, à toute heure.",
                nombre_lisible(allumer_sous.unwrap_or(0.0))
            ));
        }
        if lux_actif {
            phrases.push(format!(
                "Éteint quand il fait plus de {} lux ({} min de confirmation).",
                nombre_lisible(eteindre.or(allumer_sous).unwrap_or(0.0)),
                ligne.try_get::<i64, _>("lux_temporisation_min").unwrap_or(5)
            ));
        }
        if let Some(minutes) = minuterie {
            phrases.push(format!(
                "Éteint automatiquement {} après l'allumage{}.",
                duree_minutes(minutes),
                if minuterie_manuel {
                    ", même allumé à la main"
                } else {
                    ""
                }
            ));
        }
        if phrases.is_empty() {
            phrases.push("Ne fait rien : aucune condition réglée.".into());
        }

        // Frise de la semaine : un segment par plage et par jour, en % de 24 h.
        let semaine: Vec<Value> = (0..7)
            .map(|j| {
                let mut segments: Vec<Value> = Vec::new();
                for plage in &plages {
                    let veille = (j + 6) % 7;
                    let mut pousser = |debut: u32, fin: u32| {
                        if fin > debut {
                            segments.push(json!({
                                "gauche": debut as f64 / 14.4,
                                "largeur": (fin - debut) as f64 / 14.4,
                            }));
                        }
                    };
                    if plage.debut == plage.fin {
                        if plage.jours[j] {
                            pousser(0, 1440);
                        }
                    } else if plage.debut < plage.fin {
                        if plage.jours[j] {
                            pousser(plage.debut, plage.fin);
                        }
                    } else {
                        if plage.jours[j] {
                            pousser(plage.debut, 1440);
                        }
                        if plage.jours[veille] {
                            pousser(0, plage.fin);
                        }
                    }
                }
                json!({
                    "jour": JOURS_COURTS[j],
                    "long": JOURS_LONGS[j],
                    "aujourdhui": j == jour,
                    "segments": segments,
                    "toute_la_journee": plages.is_empty() && lux_actif,
                })
            })
            .collect();

        let prochain = prochain_changement(&plages, jour, minute).map(|(dans, allume)| {
            let a = (minute + dans) % 1440;
            let jour_cible = (jour + ((minute + dans) / 1440) as usize) % 7;
            json!({
                "allume": allume,
                "heure": heure_depuis_minutes(a).replace(':', "h"),
                "jour": if jour_cible == jour { "aujourd'hui".to_string() }
                        else if jour_cible == (jour + 1) % 7 { "demain".to_string() }
                        else { JOURS_LONGS[jour_cible].to_string() },
            })
        });

        let formulaire_plages: Vec<Value> = plages
            .iter()
            .map(|p| {
                json!({
                    "jours": p.jours.to_vec(),
                    "debut": heure_depuis_minutes(p.debut),
                    "fin": heure_depuis_minutes(p.fin),
                })
            })
            .collect();

        let sortie_ids: Vec<Value> = sorties.iter().filter_map(|s| s.get("id").cloned()).collect();
        sortie.push(json!({
            "id": id,
            "nom": ligne.try_get::<String, _>("nom").unwrap_or_default(),
            "actif": actif,
            "essai": essai,
            "note": ligne.try_get::<Option<String>, _>("note").ok().flatten(),
            "horaires_actif": !plages.is_empty(),
            "plages": formulaire_plages,
            "dans_plage": !plages.is_empty() && dans_horaire(&plages, jour, minute),
            "lux_actif": lux_actif,
            "lux_point_id": lux_point,
            "lux_nom": ligne.try_get::<Option<String>, _>("lux_nom").ok().flatten(),
            "lux_allumer_sous": allumer_sous,
            "lux_eteindre_au_dessus": eteindre,
            "lux_temporisation_min": ligne.try_get::<i64, _>("lux_temporisation_min").unwrap_or(5),
            "lux_si_muet": ligne.try_get::<String, _>("lux_si_muet").unwrap_or_else(|_| "horaire".into()),
            "minuterie_actif": minuterie.is_some(),
            "minuterie_min": minuterie,
            "minuterie_manuel": minuterie_manuel,
            "valeur_allume": ligne.try_get::<f64, _>("valeur_allume").unwrap_or(1.0),
            "valeur_eteint": ligne.try_get::<f64, _>("valeur_eteint").unwrap_or(0.0),
            "forcage": forcage,
            "sorties": sorties,
            "sortie_ids": sortie_ids,
            "resume": phrases,
            "semaine": semaine,
            "prochain": prochain,
        }));
    }
    Ok(sortie)
}

/// Sorties qu'un programme peut commander : points pilotables avec un chemin
/// d'écriture. Les éclairages d'abord, puis les autres (un relais nommé
/// « autre » peut très bien porter une lampe).
async fn sorties_disponibles(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT p.id, p.nom, p.role, s.nom AS salle, es.programme_id, ep.nom AS programme, \
                (SELECT COUNT(*) FROM ambiance_salle a WHERE a.actif=1 AND a.pilote=1 \
                   AND trim(a.sortie_ventil)=trim(p.chemin_ecriture)) AS ventilation \
         FROM ipx_point p \
         LEFT JOIN salle s ON s.id=p.salle_id \
         LEFT JOIN eclairage_sortie es ON es.point_id=p.id \
         LEFT JOIN eclairage_programme ep ON ep.id=es.programme_id \
         WHERE p.actif=1 AND p.pilotable=1 AND p.chemin_ecriture IS NOT NULL \
           AND trim(p.chemin_ecriture)<>'' \
         ORDER BY p.role<>'eclairage', s.nom, p.nom",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| {
            json!({
                "id": l.try_get::<i64, _>("id").unwrap_or(0),
                "nom": l.try_get::<String, _>("nom").unwrap_or_default(),
                "eclairage": l.try_get::<String, _>("role").unwrap_or_default() == "eclairage",
                "salle": l.try_get::<Option<String>, _>("salle").ok().flatten(),
                "programme_id": l.try_get::<Option<i64>, _>("programme_id").ok().flatten(),
                "programme": l.try_get::<Option<String>, _>("programme").ok().flatten(),
                "ventilation": l.try_get::<i64, _>("ventilation").unwrap_or(0) > 0,
            })
        })
        .collect())
}

async fn capteurs_lux(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    generic_rows(
        pool,
        "SELECT p.id, p.nom, s.nom AS salle FROM ipx_point p LEFT JOIN salle s ON s.id=p.salle_id \
         WHERE p.actif=1 AND p.role='luminosite' ORDER BY s.nom, p.nom",
    )
    .await
}

async fn journal(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    generic_rows(
        pool,
        "SELECT j.horodatage, j.utilisateur, j.nom_point, j.valeur, j.resultat \
         FROM ipx_commande_journal j \
         WHERE j.utilisateur LIKE 'programme «%' \
            OR j.point_id IN (SELECT point_id FROM eclairage_sortie) \
         ORDER BY j.id DESC LIMIT 30",
    )
    .await
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let programmes = charger_programmes(&state.pool, None).await?;
    let sorties = sorties_disponibles(&state.pool).await?;
    let libres = sorties
        .iter()
        .filter(|s| s.get("programme_id").map(Value::is_null).unwrap_or(true))
        .count();
    let mut contexte = context(&session);
    contexte.insert("programmes".into(), Value::Array(programmes));
    contexte.insert("nb_sorties".into(), json!(sorties.len()));
    contexte.insert("nb_sorties_libres".into(), json!(libres));
    contexte.insert(
        "nb_capteurs".into(),
        json!(capteurs_lux(&state.pool).await?.len()),
    );
    contexte.insert("journal".into(), Value::Array(journal(&state.pool).await?));
    contexte.insert("lampes".into(), Value::Array(lampes(&state.pool).await?));
    contexte.insert("demon".into(), super::ambiance::etat_demon(&state.pool).await);
    contexte.insert("ok".into(), json!(parametres.get("ok")));
    let (jour, minute) = jour_et_minute();
    contexte.insert("maintenant_pct".into(), json!(minute as f64 / 14.4));
    contexte.insert("aujourdhui".into(), json!(JOURS_LONGS[jour]));
    render(&state, "ambiance_eclairage.html", Value::Object(contexte))
}

/// État en direct, pour rafraîchir la page sans la recharger.
pub(super) async fn etat_json(
    State(state): State<AppState>,
    Extension(_session): Extension<SessionData>,
) -> AppResult<Response> {
    let programmes: Vec<Value> = charger_programmes(&state.pool, None)
        .await?
        .into_iter()
        .map(|programme| {
            json!({
                "id": programme.get("id"),
                "sorties": programme.get("sorties"),
                "forcage": programme.get("forcage"),
            })
        })
        .collect();
    Ok(axum::Json(json!({
        "programmes": programmes,
        "demon": super::ambiance::etat_demon(&state.pool).await,
    }))
    .into_response())
}

async fn rendre_formulaire(
    state: &AppState,
    session: &SessionData,
    programme: Value,
    erreur: Option<String>,
) -> AppResult<Html<String>> {
    let mut contexte = context(session);
    contexte.insert("programme".into(), programme);
    contexte.insert(
        "sorties".into(),
        Value::Array(sorties_disponibles(&state.pool).await?),
    );
    contexte.insert(
        "capteurs".into(),
        Value::Array(capteurs_lux(&state.pool).await?),
    );
    contexte.insert("erreur".into(), json!(erreur));
    contexte.insert("jours_courts".into(), json!(JOURS_COURTS));
    contexte.insert("jours_longs".into(), json!(JOURS_LONGS));
    render(state, "ambiance_eclairage_form.html", Value::Object(contexte))
}

pub(super) async fn nouveau(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    // Point de départ raisonnable : tous les jours, 6h30 → 20h00, en essai.
    let programme = json!({
        "id": null,
        "nom": "",
        "actif": true,
        "essai": true,
        "horaires_actif": true,
        "plages": [{"jours": vec![true; 7], "debut": "06:30", "fin": "20:00"}],
        "lux_actif": false,
        "lux_allumer_sous": 100,
        "lux_eteindre_au_dessus": 300,
        "lux_temporisation_min": 5,
        "lux_si_muet": "horaire",
        "minuterie_actif": false,
        "minuterie_min": 30,
        "minuterie_manuel": true,
        "valeur_allume": 1,
        "valeur_eteint": 0,
        "sortie_ids": [],
    });
    rendre_formulaire(&state, &session, programme, None).await
}

pub(super) async fn modifier(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    let programme = charger_programmes(&state.pool, Some(id))
        .await?
        .into_iter()
        .next()
        .ok_or(AppError::NotFound)?;
    rendre_formulaire(&state, &session, programme, None).await
}

/// Programme saisi, validé, prêt à écrire.
struct Saisie {
    id: Option<i64>,
    nom: String,
    actif: bool,
    essai: bool,
    note: Option<String>,
    plages: Vec<Plage>,
    lux: Option<(i64, f64, f64, i64, String)>,
    minuterie: Option<(i64, bool)>,
    valeur_allume: f64,
    valeur_eteint: f64,
    sorties: Vec<i64>,
}

fn coche(form: &HashMap<String, String>, cle: &str) -> bool {
    form.get(cle).is_some_and(|v| !v.is_empty() && v != "0")
}

/// Relit la saisie telle quelle, pour réafficher le formulaire après une
/// erreur sans rien faire perdre.
fn saisie_brute(form: &HashMap<String, String>) -> Value {
    let mut plages = Vec::new();
    for i in 0..PLAGES_MAX {
        let Some(debut) = form.get(&format!("plage_{i}_debut")) else {
            continue;
        };
        let jours: Vec<bool> = (0..7).map(|j| coche(form, &format!("plage_{i}_j{j}"))).collect();
        plages.push(json!({
            "jours": jours,
            "debut": debut,
            "fin": form.get(&format!("plage_{i}_fin")).cloned().unwrap_or_default(),
        }));
    }
    let sortie_ids: Vec<i64> = form
        .keys()
        .filter_map(|cle| cle.strip_prefix("sortie_")?.parse().ok())
        .collect();
    json!({
        "id": form_i64(form, "id"),
        "nom": form.get("nom").cloned().unwrap_or_default(),
        "note": form.get("note"),
        "actif": coche(form, "actif"),
        "essai": coche(form, "essai"),
        "horaires_actif": coche(form, "horaires_actif"),
        "plages": plages,
        "lux_actif": coche(form, "lux_actif"),
        "lux_point_id": form_i64(form, "lux_point_id"),
        "lux_allumer_sous": form.get("lux_allumer_sous"),
        "lux_eteindre_au_dessus": form.get("lux_eteindre_au_dessus"),
        "lux_temporisation_min": form.get("lux_temporisation_min"),
        "lux_si_muet": form.get("lux_si_muet").cloned().unwrap_or_else(|| "horaire".into()),
        "minuterie_actif": coche(form, "minuterie_actif"),
        "minuterie_min": form.get("minuterie_min"),
        "minuterie_manuel": coche(form, "minuterie_manuel"),
        "valeur_allume": form.get("valeur_allume"),
        "valeur_eteint": form.get("valeur_eteint"),
        "sortie_ids": sortie_ids,
    })
}

fn valider(form: &HashMap<String, String>) -> Result<Saisie, String> {
    let nom = form_text(form, "nom").ok_or("Donnez un nom au programme.")?;
    let mut plages = Vec::new();
    if coche(form, "horaires_actif") {
        for i in 0..PLAGES_MAX {
            let Some(debut) = form.get(&format!("plage_{i}_debut")) else {
                continue;
            };
            let fin = form.get(&format!("plage_{i}_fin")).cloned().unwrap_or_default();
            let numero = plages.len() + 1;
            let debut = minutes_depuis_heure(debut)
                .ok_or(format!("Plage {numero} : heure d'allumage illisible."))?;
            let fin = minutes_depuis_heure(&fin)
                .ok_or(format!("Plage {numero} : heure d'extinction illisible."))?;
            let mut jours = [false; 7];
            for (j, jour) in jours.iter_mut().enumerate() {
                *jour = coche(form, &format!("plage_{i}_j{j}"));
            }
            if !jours.iter().any(|j| *j) {
                return Err(format!("Plage {numero} : cochez au moins un jour."));
            }
            plages.push(Plage { jours, debut, fin });
        }
        if plages.is_empty() {
            return Err("Horaires cochés mais aucune plage : ajoutez-en une ou décochez « Horaires ».".into());
        }
    }

    let lux = if coche(form, "lux_actif") {
        let point = form_i64(form, "lux_point_id").ok_or("Choisissez le capteur de luminosité.")?;
        let allumer = form_f64(form, "lux_allumer_sous")
            .filter(|v| *v >= 0.0)
            .ok_or("Indiquez sous combien de lux il faut allumer.")?;
        let eteindre = form_f64(form, "lux_eteindre_au_dessus").unwrap_or(allumer);
        if eteindre < allumer {
            return Err(format!(
                "Le seuil d'extinction ({} lux) doit être au moins égal au seuil d'allumage ({} lux) — sinon la lampe clignoterait.",
                nombre_lisible(eteindre),
                nombre_lisible(allumer)
            ));
        }
        let tempo = form_i64(form, "lux_temporisation_min").unwrap_or(5).clamp(0, 120);
        let si_muet = match form.get("lux_si_muet").map(String::as_str) {
            Some("eteindre") => "eteindre",
            Some("garder") => "garder",
            _ => "horaire",
        };
        Some((point, allumer, eteindre, tempo, si_muet.to_string()))
    } else {
        None
    };

    let minuterie = if coche(form, "minuterie_actif") {
        let minutes = form_i64(form, "minuterie_min")
            .filter(|m| (1..=1440).contains(m))
            .ok_or("Minuterie : indiquez une durée entre 1 et 1440 minutes.")?;
        Some((minutes, coche(form, "minuterie_manuel")))
    } else {
        None
    };

    if plages.is_empty() && lux.is_none() && minuterie.is_none() {
        return Err("Choisissez au moins une façon d'agir : horaires, luminosité ou minuterie.".into());
    }

    let mut sorties: Vec<i64> = form
        .keys()
        .filter_map(|cle| cle.strip_prefix("sortie_")?.parse().ok())
        .collect();
    sorties.sort_unstable();
    sorties.dedup();
    if sorties.is_empty() {
        return Err("Cochez au moins une lampe à commander.".into());
    }

    let valeur_allume = form_f64(form, "valeur_allume").unwrap_or(1.0);
    let valeur_eteint = form_f64(form, "valeur_eteint").unwrap_or(0.0);
    if (valeur_allume - valeur_eteint).abs() < f64::EPSILON {
        return Err("Les valeurs « allumé » et « éteint » doivent être différentes.".into());
    }

    Ok(Saisie {
        id: form_i64(form, "id"),
        nom,
        actif: coche(form, "actif"),
        essai: coche(form, "essai"),
        note: form_text(form, "note"),
        plages,
        lux,
        minuterie,
        valeur_allume,
        valeur_eteint,
        sorties,
    })
}

/// Contrôles qui demandent la base : sorties pilotables, libres, hors
/// ventilation ; capteur existant.
async fn verifier_en_base(pool: &SqlitePool, saisie: &Saisie) -> AppResult<Result<(), String>> {
    for point_id in &saisie.sorties {
        let ligne: Option<(String, i64, Option<String>)> = sqlx::query_as(
            "SELECT nom, pilotable, chemin_ecriture FROM ipx_point WHERE id=? AND actif=1",
        )
        .bind(point_id)
        .fetch_optional(pool)
        .await?;
        let Some((nom, pilotable, chemin)) = ligne else {
            return Ok(Err("Une des lampes cochées n'existe plus dans l'inventaire.".into()));
        };
        let Some(chemin) = chemin.filter(|c| !c.trim().is_empty()).filter(|_| pilotable != 0) else {
            return Ok(Err(format!("« {nom} » n'est pas une sortie pilotable.")));
        };
        let autre: Option<String> = sqlx::query_scalar(
            "SELECT p.nom FROM eclairage_sortie s JOIN eclairage_programme p ON p.id=s.programme_id \
             WHERE s.point_id=? AND (? IS NULL OR s.programme_id<>?)",
        )
        .bind(point_id)
        .bind(saisie.id)
        .bind(saisie.id)
        .fetch_optional(pool)
        .await?;
        if let Some(autre) = autre {
            return Ok(Err(format!(
                "« {nom} » est déjà commandée par le programme « {autre} ». Une lampe ne suit qu'un seul programme."
            )));
        }
        let ventilation: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM ambiance_salle WHERE actif=1 AND pilote=1 AND trim(sortie_ventil)=?",
        )
        .bind(chemin.trim())
        .fetch_one(pool)
        .await?;
        if ventilation > 0 {
            return Ok(Err(format!(
                "« {nom} » est la sortie de ventilation d'une salle pilotée : impossible de la programmer en éclairage."
            )));
        }
    }
    if let Some((point, ..)) = &saisie.lux {
        let existe: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ipx_point WHERE id=?")
            .bind(point)
            .fetch_one(pool)
            .await?;
        if existe == 0 {
            return Ok(Err("Le capteur de luminosité choisi n'existe plus.".into()));
        }
    }
    Ok(Ok(()))
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let saisie = match valider(&form) {
        Ok(saisie) => saisie,
        Err(message) => {
            return Ok(rendre_formulaire(&state, &session, saisie_brute(&form), Some(message))
                .await?
                .into_response())
        }
    };
    if let Err(message) = verifier_en_base(&state.pool, &saisie).await? {
        return Ok(rendre_formulaire(&state, &session, saisie_brute(&form), Some(message))
            .await?
            .into_response());
    }

    let mut tx = state.pool.begin().await?;
    let (lux_point, lux_allumer, lux_eteindre, lux_tempo, lux_muet) = match &saisie.lux {
        Some((point, allumer, eteindre, tempo, muet)) => {
            (Some(*point), Some(*allumer), Some(*eteindre), *tempo, muet.clone())
        }
        None => (None, None, None, 5, "horaire".to_string()),
    };
    let (minuterie, minuterie_manuel) = match saisie.minuterie {
        Some((minutes, manuel)) => (Some(minutes), manuel),
        None => (None, true),
    };
    let id = match saisie.id {
        Some(id) => {
            let modifie = sqlx::query(
                "UPDATE eclairage_programme SET nom=?, actif=?, essai=?, note=?, lux_point_id=?, \
                 lux_allumer_sous=?, lux_eteindre_au_dessus=?, lux_temporisation_min=?, lux_si_muet=?, \
                 minuterie_min=?, minuterie_manuel=?, valeur_allume=?, valeur_eteint=?, \
                 modifie_le=datetime('now') WHERE id=?",
            )
            .bind(&saisie.nom)
            .bind(i64::from(saisie.actif))
            .bind(i64::from(saisie.essai))
            .bind(&saisie.note)
            .bind(lux_point)
            .bind(lux_allumer)
            .bind(lux_eteindre)
            .bind(lux_tempo)
            .bind(&lux_muet)
            .bind(minuterie)
            .bind(i64::from(minuterie_manuel))
            .bind(saisie.valeur_allume)
            .bind(saisie.valeur_eteint)
            .bind(id)
            .execute(&mut *tx)
            .await?;
            if modifie.rows_affected() == 0 {
                return Err(AppError::NotFound);
            }
            id
        }
        None => sqlx::query(
            "INSERT INTO eclairage_programme(nom,actif,essai,note,lux_point_id,lux_allumer_sous,\
             lux_eteindre_au_dessus,lux_temporisation_min,lux_si_muet,minuterie_min,minuterie_manuel,\
             valeur_allume,valeur_eteint) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?)",
        )
        .bind(&saisie.nom)
        .bind(i64::from(saisie.actif))
        .bind(i64::from(saisie.essai))
        .bind(&saisie.note)
        .bind(lux_point)
        .bind(lux_allumer)
        .bind(lux_eteindre)
        .bind(lux_tempo)
        .bind(&lux_muet)
        .bind(minuterie)
        .bind(i64::from(minuterie_manuel))
        .bind(saisie.valeur_allume)
        .bind(saisie.valeur_eteint)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid(),
    };

    sqlx::query("DELETE FROM eclairage_plage WHERE programme_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for plage in &saisie.plages {
        sqlx::query("INSERT INTO eclairage_plage(programme_id,jours,debut,fin) VALUES(?,?,?,?)")
            .bind(id)
            .bind(jours_en_texte(&plage.jours))
            .bind(i64::from(plage.debut))
            .bind(i64::from(plage.fin))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM eclairage_sortie WHERE programme_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for point_id in &saisie.sorties {
        sqlx::query("INSERT INTO eclairage_sortie(programme_id,point_id) VALUES(?,?)")
            .bind(id)
            .bind(point_id)
            .execute(&mut *tx)
            .await?;
    }
    // Mémoire du démon effacée : les nouveaux réglages s'appliquent dès le
    // prochain cycle, au lieu d'attendre le prochain changement d'état.
    sqlx::query("DELETE FROM eclairage_etat WHERE programme_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;

    let message = format!(
        "Programme « {} » enregistré{}",
        saisie.nom,
        if !saisie.actif {
            " (en pause)"
        } else if saisie.essai {
            " en mode essai : rien n'est envoyé aux lampes"
        } else {
            " : il s'applique dans les 30 secondes"
        }
    );
    Ok(Redirect::to(&format!(
        "/ambiance/eclairage?ok={}#programme-{id}",
        encode_query(&message)
    ))
    .into_response())
}

async fn nom_programme(pool: &SqlitePool, id: i64) -> AppResult<String> {
    sqlx::query_scalar("SELECT nom FROM eclairage_programme WHERE id=?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound)
}

fn retour(message: &str, id: Option<i64>) -> Response {
    let ancre = id.map(|id| format!("#programme-{id}")).unwrap_or_default();
    Redirect::to(&format!(
        "/ambiance/eclairage?ok={}{ancre}",
        encode_query(message)
    ))
    .into_response()
}

/// Met en pause ou relance un programme.
pub(super) async fn basculer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = nom_programme(&state.pool, id).await?;
    let actif = coche(&form, "actif");
    let essai = form.get("essai").map(|v| v != "0");
    sqlx::query(
        "UPDATE eclairage_programme SET actif=?, essai=COALESCE(?,essai), \
         modifie_le=datetime('now') WHERE id=?",
    )
    .bind(i64::from(actif))
    .bind(essai.map(i64::from))
    .bind(id)
    .execute(&state.pool)
    .await?;
    sqlx::query("DELETE FROM eclairage_etat WHERE programme_id=?")
        .bind(id)
        .execute(&state.pool)
        .await?;
    let message = match (actif, essai) {
        (false, _) => format!("« {nom} » en pause : les lampes restent dans leur état actuel"),
        (true, Some(false)) => format!("« {nom} » commande maintenant les lampes pour de vrai"),
        (true, Some(true)) => format!("« {nom} » repasse en mode essai"),
        (true, None) => format!("« {nom} » relancé"),
    };
    Ok(retour(&message, Some(id)))
}

/// Forçage manuel : allumé ou éteint pendant une durée, puis retour au
/// programme. Appliqué par le démon au cycle suivant.
pub(super) async fn forcer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = nom_programme(&state.pool, id).await?;
    let message = match form.get("etat").map(String::as_str) {
        Some("reprendre") => {
            sqlx::query("UPDATE eclairage_programme SET forcage=NULL, forcage_fin=NULL WHERE id=?")
                .bind(id)
                .execute(&state.pool)
                .await?;
            format!("« {nom} » reprend son programme")
        }
        Some(etat @ ("allumer" | "eteindre")) => {
            let minutes = form_i64(&form, "duree_min")
                .filter(|m| (1..=7 * 1440).contains(m))
                .ok_or_else(|| AppError::Invalid("Durée du forçage invalide".into()))?;
            let allume = etat == "allumer";
            sqlx::query("UPDATE eclairage_programme SET forcage=?, forcage_fin=? WHERE id=?")
                .bind(i64::from(allume))
                .bind(maintenant() + minutes * 60)
                .bind(id)
                .execute(&state.pool)
                .await?;
            format!(
                "« {nom} » forcé {} pendant {} — appliqué dans les 30 secondes",
                if allume { "allumé" } else { "éteint" },
                duree_minutes(minutes)
            )
        }
        _ => return Err(AppError::Invalid("Action de forçage inconnue".into())),
    };
    Ok(retour(&message, Some(id)))
}

pub(super) async fn dupliquer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    nom_programme(&state.pool, id).await?;
    let mut tx = state.pool.begin().await?;
    // La copie part en pause, en essai et sans lampe : une lampe ne suit
    // qu'un programme, c'est à l'éleveur de choisir lesquelles lui donner.
    let nouveau = sqlx::query(
        "INSERT INTO eclairage_programme(nom,actif,essai,note,lux_point_id,lux_allumer_sous,\
         lux_eteindre_au_dessus,lux_temporisation_min,lux_si_muet,minuterie_min,minuterie_manuel,\
         valeur_allume,valeur_eteint) \
         SELECT nom || ' (copie)',0,1,note,lux_point_id,lux_allumer_sous,lux_eteindre_au_dessus,\
         lux_temporisation_min,lux_si_muet,minuterie_min,minuterie_manuel,valeur_allume,valeur_eteint \
         FROM eclairage_programme WHERE id=?",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO eclairage_plage(programme_id,jours,debut,fin) \
         SELECT ?,jours,debut,fin FROM eclairage_plage WHERE programme_id=? ORDER BY id",
    )
    .bind(nouveau)
    .bind(id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Redirect::to(&format!("/ambiance/eclairage/{nouveau}")).into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = nom_programme(&state.pool, id).await?;
    let mut tx = state.pool.begin().await?;
    for sql in [
        "DELETE FROM eclairage_etat WHERE programme_id=?",
        "DELETE FROM eclairage_sortie WHERE programme_id=?",
        "DELETE FROM eclairage_plage WHERE programme_id=?",
        "DELETE FROM eclairage_programme WHERE id=?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(retour(
        &format!("Programme « {nom} » supprimé : ses lampes restent dans leur état actuel"),
        None,
    ))
}

// ---------------------------------------------------------------------------
// Lampes : déclarer un relais comme éclairage sans connaître l'API
//
// On lit la liste des sorties commandables sur l'automate, l'éleveur
// reconnaît la sienne à son nom, l'essaie (Allumer / Éteindre) et
// l'enregistre. Le chemin d'API est déduit, jamais saisi.
// ---------------------------------------------------------------------------

/// Chemin d'un objet tout-ou-rien de l'IPX : `api/core/io/<nombre>`.
fn chemin_io_valide(chemin: &str) -> bool {
    chemin
        .strip_prefix("api/core/io/")
        .is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()))
}

/// Lampes déclarées (rôle éclairage), y compris celles mal configurées, pour
/// qu'on puisse les voir et les supprimer.
async fn lampes(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let lignes = sqlx::query(
        "SELECT p.id, p.nom, p.chemin, p.chemin_ecriture, p.pilotable, s.nom AS salle, \
                ep.nom AS programme \
         FROM ipx_point p LEFT JOIN salle s ON s.id=p.salle_id \
         LEFT JOIN eclairage_sortie es ON es.point_id=p.id \
         LEFT JOIN eclairage_programme ep ON ep.id=es.programme_id \
         WHERE p.role='eclairage' AND p.actif=1 ORDER BY s.nom, p.nom",
    )
    .fetch_all(pool)
    .await?;
    Ok(lignes
        .into_iter()
        .map(|l| {
            let ecriture: Option<String> = l.try_get("chemin_ecriture").ok().flatten();
            let pilotable = l.try_get::<i64, _>("pilotable").unwrap_or(0) != 0;
            let valide = pilotable
                && ecriture
                    .as_deref()
                    .is_some_and(|c| !c.trim().is_empty() && !c.trim_start().starts_with('['));
            json!({
                "id": l.try_get::<i64, _>("id").unwrap_or(0),
                "nom": l.try_get::<String, _>("nom").unwrap_or_default(),
                "salle": l.try_get::<Option<String>, _>("salle").ok().flatten(),
                "programme": l.try_get::<Option<String>, _>("programme").ok().flatten(),
                "valide": valide,
            })
        })
        .collect())
}

/// Choisir le relais d'une nouvelle lampe.
pub(super) async fn lampe_nouvelle(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(parametres): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    require_writer(&session)?;
    let hotes = super::ambiance::hotes_connus(&state.pool).await;
    let hote = parametres
        .get("hote")
        .filter(|h| hotes.contains(h))
        .cloned()
        .or_else(|| hotes.first().cloned())
        .unwrap_or_default();
    let sorties = if hote.is_empty() {
        None
    } else {
        super::ambiance::sorties_commandables(&state.pool, &hote).await?
    };
    // Ce qui est déjà déclaré, par chemin : nom et rôle choisis par l'éleveur.
    let connus: HashMap<String, (String, String)> = sqlx::query_as::<_, (String, String, String)>(
        "SELECT chemin, nom, role FROM ipx_point WHERE hote=?",
    )
    .bind(&hote)
    .fetch_all(&state.pool)
    .await?
    .into_iter()
    .map(|(chemin, nom, role)| (chemin, (nom, role)))
    .collect();
    let sorties: Option<Vec<Value>> = sorties.map(|liste| {
        liste
            .into_iter()
            .map(|mut s| {
                let chemin = s.get("chemin").and_then(Value::as_str).unwrap_or("").to_string();
                let nom_ipx = s.get("nom_ipx").and_then(Value::as_str).unwrap_or("");
                let probable = {
                    let bas = nom_ipx.to_lowercase();
                    bas.contains("ecl") || bas.contains("écl") || bas.contains("lum") || bas.contains("light")
                };
                if let Some(objet) = s.as_object_mut() {
                    objet.insert("probable".into(), json!(probable));
                    if let Some((nom, role)) = connus.get(&chemin) {
                        objet.insert("deja".into(), json!(nom));
                        objet.insert("deja_lampe".into(), json!(role == "eclairage"));
                    }
                }
                s
            })
            .collect()
    });
    let mut contexte = context(&session);
    contexte.insert("hote".into(), json!(hote));
    contexte.insert("hotes".into(), json!(hotes));
    contexte.insert("sorties".into(), json!(sorties));
    contexte.insert(
        "salles".into(),
        Value::Array(generic_rows(&state.pool, "SELECT id, nom FROM salle ORDER BY ordre, nom").await?),
    );
    contexte.insert("ok".into(), json!(parametres.get("ok")));
    render(&state, "ambiance_eclairage_lampe.html", Value::Object(contexte))
}

/// Allume ou éteint une sortie pour vérifier que c'est la bonne lampe :
/// soit une lampe déclarée (`point_id`), soit un relais pas encore déclaré
/// (`hote` + `chemin`). Journalisé comme toute commande.
pub(super) async fn lampe_tester(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let allumer = form.get("etat").map(String::as_str) == Some("1");
    let (hote, chemin, nom, point_id) = match form_i64(&form, "point_id") {
        Some(id) => {
            let ligne: Option<(String, Option<String>, String)> = sqlx::query_as(
                "SELECT hote, chemin_ecriture, nom FROM ipx_point WHERE id=? AND pilotable=1",
            )
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
            let (hote, chemin, nom) = ligne.ok_or(AppError::NotFound)?;
            let chemin = chemin.ok_or_else(|| AppError::Invalid("Lampe sans adresse de commande".into()))?;
            (hote, chemin, nom, Some(id))
        }
        None => {
            let hote = form_text(&form, "hote").unwrap_or_default();
            let chemin = form_text(&form, "chemin").unwrap_or_default();
            if !super::ambiance::hotes_connus(&state.pool).await.contains(&hote) || !chemin_io_valide(&chemin) {
                return Err(AppError::Invalid("Sortie inconnue".into()));
            }
            let nom = form_text(&form, "nom").unwrap_or_else(|| chemin.clone());
            (hote, chemin, nom, None)
        }
    };
    let ventilation: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM ambiance_salle WHERE actif=1 AND pilote=1 AND trim(sortie_ventil)=?",
    )
    .bind(chemin.trim())
    .fetch_one(&state.pool)
    .await?;
    if ventilation > 0 {
        return Err(AppError::Invalid(
            "Cette sortie commande la ventilation d'une salle en régulation automatique.".into(),
        ));
    }
    let valeur = if allumer { 1.0 } else { 0.0 };
    let compte_rendu = super::ambiance::envoyer_commande(&state.pool, &hote, &chemin, valeur).await?;
    sqlx::query(
        "INSERT INTO ipx_commande_journal(utilisateur,point_id,nom_point,valeur,resultat) VALUES(?,?,?,?,?)",
    )
    .bind(&session.identifiant)
    .bind(point_id)
    .bind(&nom)
    .bind(valeur)
    .bind(format!("essai de lampe : {compte_rendu}"))
    .execute(&state.pool)
    .await?;
    let message = if compte_rendu == "envoyée" {
        format!(
            "« {nom} » : commande « {} » envoyée. La lampe a-t-elle réagi ?",
            if allumer { "allumer" } else { "éteindre" }
        )
    } else {
        format!("« {nom} » : commande {compte_rendu}")
    };
    let cible = if form.get("retour").map(String::as_str) == Some("choix") {
        format!(
            "/ambiance/eclairage/lampe/nouvelle?hote={}&ok={}#sortie-{}",
            encode_query(&hote),
            encode_query(&message),
            chemin.rsplit('/').next().unwrap_or("")
        )
    } else {
        format!("/ambiance/eclairage?ok={}#lampes", encode_query(&message))
    };
    Ok(Redirect::to(&cible).into_response())
}

/// Enregistre un relais comme lampe : rôle éclairage, pilotable, chemin de
/// lecture et d'écriture identiques.
pub(super) async fn lampe_enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let hote = form_text(&form, "hote").unwrap_or_default();
    let chemin = form_text(&form, "chemin").unwrap_or_default();
    if !super::ambiance::hotes_connus(&state.pool).await.contains(&hote) || !chemin_io_valide(&chemin) {
        return Err(AppError::Invalid("Sortie inconnue".into()));
    }
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Donnez un nom à la lampe".into()))?;
    let salle_id = form_i64(&form, "salle_id");
    sqlx::query(
        "INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,salle_id,unite,pilotable,actif) \
         VALUES(?,?,?,?,'eclairage',?,'',1,1) \
         ON CONFLICT(hote,chemin) DO UPDATE SET chemin_ecriture=excluded.chemin_ecriture, \
           nom=excluded.nom, role='eclairage', salle_id=excluded.salle_id, pilotable=1, actif=1",
    )
    .bind(&hote)
    .bind(&chemin)
    .bind(&chemin)
    .bind(&nom)
    .bind(salle_id)
    .execute(&state.pool)
    .await?;
    let message = format!("Lampe « {nom} » ajoutée. Vous pouvez maintenant l'utiliser dans un programme.");
    Ok(Redirect::to(&format!("/ambiance/eclairage?ok={}#lampes", encode_query(&message))).into_response())
}

/// Retire une lampe (et elle seule : rôle éclairage). Le journal garde la
/// trace des commandes passées, sans lien vers le point supprimé.
pub(super) async fn lampe_supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom: String = sqlx::query_scalar("SELECT nom FROM ipx_point WHERE id=? AND role='eclairage'")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let mut tx = state.pool.begin().await?;
    for sql in [
        "UPDATE ipx_commande_journal SET point_id=NULL WHERE point_id=?",
        "DELETE FROM eclairage_etat WHERE point_id=?",
        "DELETE FROM eclairage_sortie WHERE point_id=?",
        "DELETE FROM ipx_point WHERE id=?",
    ] {
        sqlx::query(sql).bind(id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(retour(&format!("Lampe « {nom} » retirée"), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formulaire(paires: &[(&str, &str)]) -> HashMap<String, String> {
        paires
            .iter()
            .map(|(cle, valeur)| (cle.to_string(), valeur.to_string()))
            .collect()
    }

    #[test]
    fn les_jours_se_disent_en_francais() {
        assert_eq!(jours_lisibles(&[true; 7]), "tous les jours");
        assert_eq!(
            jours_lisibles(&[true, true, true, true, true, false, false]),
            "du lundi au vendredi"
        );
        assert_eq!(
            jours_lisibles(&[true, false, true, false, true, false, false]),
            "lundi, mercredi et vendredi"
        );
        assert_eq!(
            jours_lisibles(&[false, false, false, false, false, false, true]),
            "le dimanche"
        );
    }

    #[test]
    fn une_saisie_complete_est_acceptee() {
        let saisie = valider(&formulaire(&[
            ("nom", "Maternité"),
            ("actif", "1"),
            ("horaires_actif", "1"),
            ("plage_0_debut", "06:30"),
            ("plage_0_fin", "20:00"),
            ("plage_0_j0", "1"),
            ("plage_3_debut", "22h"),
            ("plage_3_fin", "6h"),
            ("plage_3_j5", "1"),
            ("lux_actif", "1"),
            ("lux_point_id", "9"),
            ("lux_allumer_sous", "120"),
            ("lux_eteindre_au_dessus", "300"),
            ("minuterie_actif", "1"),
            ("minuterie_min", "45"),
            ("sortie_4", "on"),
            ("sortie_2", "on"),
        ]))
        .expect("saisie valide");
        assert_eq!(saisie.plages.len(), 2);
        assert_eq!(saisie.plages[1].debut, 22 * 60);
        assert_eq!(saisie.sorties, vec![2, 4]);
        assert_eq!(saisie.lux.as_ref().map(|l| l.0), Some(9));
        assert_eq!(saisie.minuterie, Some((45, false)));
        assert!(!saisie.essai);
    }

    #[test]
    fn des_seuils_inverses_sont_refuses() {
        let erreur = valider(&formulaire(&[
            ("nom", "PS"),
            ("lux_actif", "1"),
            ("lux_point_id", "9"),
            ("lux_allumer_sous", "300"),
            ("lux_eteindre_au_dessus", "100"),
            ("sortie_1", "on"),
        ]))
        .err()
        .unwrap();
        assert!(erreur.contains("clignoterait"), "{erreur}");
    }

    #[test]
    fn il_faut_une_lampe_et_une_facon_d_agir() {
        assert!(valider(&formulaire(&[("nom", "Vide"), ("sortie_1", "on")]))
            .err()
            .unwrap()
            .contains("au moins une façon"));
        assert!(valider(&formulaire(&[
            ("nom", "Sans lampe"),
            ("minuterie_actif", "1"),
            ("minuterie_min", "10"),
        ]))
        .err()
        .unwrap()
        .contains("lampe"));
    }

    #[test]
    fn une_plage_sans_jour_est_signalee() {
        let erreur = valider(&formulaire(&[
            ("nom", "X"),
            ("horaires_actif", "1"),
            ("plage_0_debut", "06:00"),
            ("plage_0_fin", "07:00"),
            ("sortie_1", "on"),
        ]))
        .err()
        .unwrap();
        assert!(erreur.contains("au moins un jour"), "{erreur}");
    }

    /// Parcours complet sur un état donné : création, erreurs de saisie,
    /// lampe déjà prise, forçage, mise en service, duplication, suppression.
    /// Renvoie l'identifiant du programme conservé.
    async fn parcours(state: AppState, lux_point: i64) -> anyhow::Result<i64> {
        use super::super::documents::tests::session;
        let sortie = sqlx::query(
            "INSERT INTO ipx_point(hote,chemin,chemin_ecriture,nom,role,pilotable) \
             VALUES('ipx-test.invalid','api/core/io/65536','api/core/io/65536','Éclairage essai','eclairage',1)",
        )
        .execute(&state.pool)
        .await?
        .last_insert_rowid();
        let corps = |r: Response| async move {
            let octets = axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap();
            String::from_utf8(octets.to_vec()).unwrap()
        };
        let avec_csrf = |paires: &[(&str, &str)]| {
            let mut f = formulaire(paires);
            f.insert("csrf_token".into(), "test".into());
            f
        };

        let liste = page(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(liste.contains("Éclairage automatique"));
        let vierge = nouveau(State(state.clone()), Extension(session())).await?.0;
        assert!(vierge.contains("Éclairage essai") && vierge.contains("06:30"));

        // Saisie incomplète : le formulaire revient avec le message et la saisie.
        let r = enregistrer(
            State(state.clone()),
            Extension(session()),
            Form(avec_csrf(&[("nom", "Maternité jour"), ("minuterie_actif", "1"), ("minuterie_min", "20")])),
        )
        .await?;
        let html = corps(r).await;
        assert!(html.contains("Cochez au moins une lampe") && html.contains("Maternité jour"), "{html}");

        let sortie_cle = format!("sortie_{sortie}");
        let lux = lux_point.to_string();
        let champs = [
            ("nom", "Maternité jour"),
            ("actif", "1"),
            ("essai", "1"),
            ("horaires_actif", "1"),
            ("plage_0_debut", "06:30"),
            ("plage_0_fin", "20:00"),
            ("plage_0_j0", "1"),
            ("plage_0_j1", "1"),
            ("plage_1_debut", "22:00"),
            ("plage_1_fin", "02:00"),
            ("plage_1_j5", "1"),
            ("lux_actif", "1"),
            ("lux_point_id", lux.as_str()),
            ("lux_allumer_sous", "100"),
            ("lux_eteindre_au_dessus", "300"),
            ("minuterie_actif", "1"),
            ("minuterie_min", "90"),
            (sortie_cle.as_str(), "on"),
        ];
        let r = enregistrer(State(state.clone()), Extension(session()), Form(avec_csrf(&champs))).await?;
        assert!(r.status().is_redirection(), "{}", r.status());
        let id: i64 = sqlx::query_scalar("SELECT id FROM eclairage_programme WHERE nom='Maternité jour'")
            .fetch_one(&state.pool)
            .await?;
        let plages: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM eclairage_plage WHERE programme_id=?")
            .bind(id)
            .fetch_one(&state.pool)
            .await?;
        assert_eq!(plages, 2);

        let liste = page(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(liste.contains("Maternité jour") && liste.contains("Mode essai"));
        assert!(liste.contains("22h00 à 02h00 le lendemain"), "résumé");
        let fiche = modifier(State(state.clone()), Extension(session()), Path(id)).await?.0;
        assert!(fiche.contains("value=\"22:00\"") && fiche.contains(" checked"));

        // Une lampe ne suit qu'un programme.
        let mut autre: Vec<(&str, &str)> = champs.to_vec();
        autre[0] = ("nom", "Concurrent");
        let html = corps(enregistrer(State(state.clone()), Extension(session()), Form(avec_csrf(&autre))).await?).await;
        assert!(html.contains("déjà commandée"), "{html}");

        forcer(State(state.clone()), Extension(session()), Path(id),
            Form(avec_csrf(&[("etat", "allumer"), ("duree_min", "60")]))).await?;
        let forcage: Option<i64> = sqlx::query_scalar("SELECT forcage FROM eclairage_programme WHERE id=?")
            .bind(id).fetch_one(&state.pool).await?;
        assert_eq!(forcage, Some(1));
        forcer(State(state.clone()), Extension(session()), Path(id),
            Form(avec_csrf(&[("etat", "reprendre")]))).await?;

        basculer(State(state.clone()), Extension(session()), Path(id),
            Form(avec_csrf(&[("actif", "1"), ("essai", "0")]))).await?;
        basculer(State(state.clone()), Extension(session()), Path(id),
            Form(avec_csrf(&[("actif", "1"), ("essai", "1")]))).await?;
        let essai: i64 = sqlx::query_scalar("SELECT essai FROM eclairage_programme WHERE id=?")
            .bind(id).fetch_one(&state.pool).await?;
        assert_eq!(essai, 1);

        dupliquer(State(state.clone()), Extension(session()), Path(id), Form(avec_csrf(&[]))).await?;
        let copie: i64 = sqlx::query_scalar("SELECT id FROM eclairage_programme WHERE nom='Maternité jour (copie)' AND actif=0")
            .fetch_one(&state.pool).await?;
        let sorties_copie: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM eclairage_sortie WHERE programme_id=?")
            .bind(copie).fetch_one(&state.pool).await?;
        assert_eq!(sorties_copie, 0, "la copie ne reprend pas les lampes");
        supprimer(State(state.clone()), Extension(session()), Path(copie), Form(avec_csrf(&[]))).await?;

        let etat = etat_json(State(state.clone()), Extension(session())).await?;
        assert!(corps(etat).await.contains("\"programmes\""));
        Ok(id)
    }

    #[tokio::test]
    async fn ajouter_une_lampe_sans_connaitre_l_api() -> anyhow::Result<()> {
        use super::super::documents::tests::{session, state};
        let state = state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&state.pool).await?;
        }
        sqlx::raw_sql(
            "CREATE TABLE IF NOT EXISTS ipx_section(id INTEGER PRIMARY KEY, nom TEXT, source TEXT, motif TEXT, \
             id_debut INTEGER, id_fin INTEGER, commandable INTEGER, ordre INTEGER, note TEXT); \
             INSERT INTO parametre(cle,valeur) VALUES('ambiance_ipx_hote','ipx-test.invalid');",
        )
        .execute(&state.pool)
        .await?;
        let f = |paires: &[(&str, &str)]| {
            let mut f = formulaire(paires);
            f.insert("csrf_token".into(), "test".into());
            f
        };

        // Automate injoignable : la page le dit au lieu d'afficher une liste vide.
        let html = lampe_nouvelle(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(html.contains("ne répond pas"), "{html}");

        // Une adresse qui n'est pas celle d'un relais est refusée.
        let refus = lampe_enregistrer(State(state.clone()), Extension(session()),
            Form(f(&[("hote", "ipx-test.invalid"), ("chemin", "[1].name"), ("nom", "X")]))).await;
        assert!(refus.is_err());
        let refus = lampe_enregistrer(State(state.clone()), Extension(session()),
            Form(f(&[("hote", "autre.invalid"), ("chemin", "api/core/io/65536"), ("nom", "X")]))).await;
        assert!(refus.is_err(), "hôte inconnu");

        lampe_enregistrer(State(state.clone()), Extension(session()),
            Form(f(&[("hote", "ipx-test.invalid"), ("chemin", "api/core/io/65537"), ("nom", "Verraterie")]))).await?;
        let (role, pilotable, ecriture): (String, i64, String) = sqlx::query_as(
            "SELECT role, pilotable, chemin_ecriture FROM ipx_point WHERE chemin='api/core/io/65537'",
        )
        .fetch_one(&state.pool)
        .await?;
        assert_eq!((role.as_str(), pilotable, ecriture.as_str()), ("eclairage", 1, "api/core/io/65537"));

        let liste = page(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        assert!(liste.contains("Mes lampes") && liste.contains("Verraterie") && !liste.contains("Mal configurée"));
        let fiche = nouveau(State(state.clone()), Extension(session())).await?.0;
        assert!(fiche.contains("Verraterie"), "la lampe est proposée dans un programme");

        let id: i64 = sqlx::query_scalar("SELECT id FROM ipx_point WHERE chemin='api/core/io/65537'")
            .fetch_one(&state.pool).await?;
        lampe_supprimer(State(state.clone()), Extension(session()), Path(id), Form(f(&[]))).await?;
        let reste: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ipx_point").fetch_one(&state.pool).await?;
        assert_eq!(reste, 0);
        Ok(())
    }

    #[test]
    fn seul_un_chemin_de_relais_est_accepte() {
        assert!(chemin_io_valide("api/core/io/65536"));
        assert!(!chemin_io_valide("[1].name"));
        assert!(!chemin_io_valide("api/core/io/"));
        assert!(!chemin_io_valide("api/core/ana/262204"));
        assert!(!chemin_io_valide("api/core/io/1/../../x"));
    }

    #[tokio::test]
    async fn parcours_complet_sur_base_de_test() -> anyhow::Result<()> {
        let state = super::super::documents::tests::state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&state.pool).await?;
        }
        let lux = sqlx::query("INSERT INTO ipx_point(hote,chemin,nom,role) VALUES('ipx-test.invalid','api/core/ana/1','Lux','luminosite')")
            .execute(&state.pool).await?.last_insert_rowid();
        parcours(state, lux).await?;
        Ok(())
    }

    /// Même parcours sur une copie de la base réelle :
    /// `EO_TEST_BASE=/var/tmp/copie/elevage.db EO_TEST_LUX=9 cargo test -- --ignored parcours_complet_sur_copie`.
    /// Le programme créé est laissé en place (en essai) pour essayer eo-regul
    /// sur la même copie.
    #[tokio::test]
    #[ignore]
    async fn parcours_complet_sur_copie_de_la_base_reelle() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let lux: i64 = std::env::var("EO_TEST_LUX")?.parse()?;
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&chemin)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool,
            crate::templates::build()?,
        );
        parcours(state, lux).await?;
        Ok(())
    }
}

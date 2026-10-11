//! Plan de vaccination en frise (/sanitaire/plan) : une ligne par catégorie
//! (cochettes, truies, porcelets, verrats), les stades en barres colorées
//! comme la frise des bandes, les vaccins et soins en pastilles que l'on
//! glisse au jour voulu. Lit et écrit les actes du plan sanitaire
//! (`acteprotocole`) : la conduite de la semaine et les soins de portée
//! s'en servent déjà, rien n'est dupliqué.
//!
//! Repères (J0) par ligne : cochettes = arrivée (référence `arrivee`),
//! truies = IA, porcelets = naissance (référence `mise_bas`, ce qui alimente
//! `soinportee`), verrats = jour de l'année (référence `annee`). Un acte
//! truie existant calé sur la mise-bas ou le sevrage garde sa référence :
//! seul son `jour` est recalculé quand on le déplace.

use super::*;
use serde::Deserialize;

/// Ligne de la frise d'après la cible de l'acte.
fn ligne_de(cible: &str) -> Option<&'static str> {
    let c = cible.trim().to_lowercase();
    if c.contains("cochette") {
        Some("cochettes")
    } else if c.contains("porcelet") {
        Some("porcelets")
    } else if c.contains("verrat") {
        Some("verrats")
    } else if c.contains("femelle") || c.contains("truie") {
        Some("truies")
    } else {
        None
    }
}

/// Position sur la frise (jours depuis le J0 de la ligne) du point de
/// référence de l'acte ; `None` si la référence n'a pas de sens sur la ligne.
fn base(ligne: &str, reference: &str, c: &BandSchedule) -> Option<i64> {
    let r = reference.trim().to_lowercase();
    match ligne {
        "truies" => match r.as_str() {
            "gestation" | "ia" | "insemination" => Some(0),
            "mise_bas" | "misebas" | "mb" | "lactation" => Some(c.gestation),
            "sevrage" => Some(c.gestation + c.weaning),
            _ => None,
        },
        "porcelets" => match r.as_str() {
            "mise_bas" | "misebas" | "naissance" => Some(0),
            "sevrage" => Some(c.weaning),
            _ => None,
        },
        "cochettes" | "verrats" => Some(0),
        _ => None,
    }
}

fn nouvelle_cible_reference(ligne: &str) -> Option<(&'static str, &'static str)> {
    match ligne {
        "cochettes" => Some(("Cochette", "arrivee")),
        "truies" => Some(("toutes_femelles", "ia")),
        "porcelets" => Some(("Porcelet", "mise_bas")),
        "verrats" => Some(("Verrat", "annee")),
        _ => None,
    }
}

type Acte = (i64, String, String, String, i64, Option<String>, Option<String>, Option<String>);

async fn actes(pool: &SqlitePool) -> AppResult<Vec<Acte>> {
    Ok(sqlx::query_as(
        "SELECT id,libelle,cible,reference,jour,produit,categorie,dose FROM acteprotocole WHERE actif=1 ORDER BY id",
    )
    .fetch_all(pool)
    .await?)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(q): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let c = load_band_schedule(&state.pool).await?;
    let mut lignes = serde_json::Map::new();
    for l in ["cochettes", "truies", "porcelets", "verrats"] {
        lignes.insert(l.into(), json!([]));
    }
    for (id, libelle, cible, reference, jour, produit, categorie, dose) in actes(&state.pool).await? {
        let Some(l) = ligne_de(&cible) else { continue };
        let Some(b) = base(l, &reference, &c) else { continue };
        if let Some(Value::Array(v)) = lignes.get_mut(l) {
            v.push(json!({"id": id, "nom": libelle, "produit": produit.unwrap_or_default(),
                "type": if categorie.as_deref() == Some("vaccin") { "v" } else { "s" },
                "dose": dose.unwrap_or_default(), "x": b + jour}));
        }
    }
    let cycle = json!({"gestation": c.gestation, "mater_avant": c.maternity_before_farrowing,
        "sevrage": c.weaning, "transfert": c.transfer_finishing, "depart": c.departure});
    let mut ctx = context(&session);
    ctx.insert("plan".into(), json!({"lignes": lignes, "cycle": cycle}));
    ctx.insert("enregistre".into(), json!(q.contains_key("ok")));
    render(&state, "sanitaire_plan.html", Value::Object(ctx))
}

#[derive(Deserialize)]
struct Pastille {
    id: Option<i64>,
    ligne: String,
    nom: String,
    #[serde(default)]
    produit: String,
    #[serde(default)]
    dose: String,
    #[serde(rename = "type")]
    type_: String,
    x: i64,
}

fn vide(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

pub(super) async fn enregistrer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pastilles: Vec<Pastille> = serde_json::from_str(form.get("plan").map(String::as_str).unwrap_or("[]"))
        .map_err(|e| AppError::Invalid(format!("Plan illisible : {e}")))?;
    let c = load_band_schedule(&state.pool).await?;
    // Actes affichés sur la frise : ceux qui n'y sont plus sont désactivés
    // (pas supprimés, l'historique des actes réalisés y fait référence).
    let mut affiches: HashMap<i64, (String, String)> = HashMap::new();
    for (id, _, cible, reference, ..) in actes(&state.pool).await? {
        if let Some(l) = ligne_de(&cible) {
            if base(l, &reference, &c).is_some() {
                affiches.insert(id, (l.to_string(), reference));
            }
        }
    }
    let mut tx = state.pool.begin().await?;
    let mut touches = Vec::new();
    let mut gardes = std::collections::HashSet::new();
    for p in &pastilles {
        let nom = p.nom.trim();
        if nom.is_empty() {
            continue;
        }
        let categorie = if p.type_ == "v" { "vaccin" } else { "traitement" };
        match p.id.and_then(|id| affiches.get(&id).map(|a| (id, a))) {
            Some((id, (ligne, reference))) => {
                let jour = p.x - base(ligne, reference, &c).unwrap_or(0);
                sqlx::query("UPDATE acteprotocole SET libelle=?,produit=?,dose=?,categorie=?,jour=? WHERE id=?")
                    .bind(nom).bind(vide(&p.produit)).bind(vide(&p.dose)).bind(categorie).bind(jour).bind(id)
                    .execute(&mut *tx).await?;
                gardes.insert(id);
                touches.push(id);
            }
            None => {
                let Some((cible, reference)) = nouvelle_cible_reference(&p.ligne) else { continue };
                let r = sqlx::query("INSERT INTO acteprotocole(libelle,cible,reference,jour,produit,dose,categorie,actif,rappel) VALUES(?,?,?,?,?,?,?,1,0)")
                    .bind(nom).bind(cible).bind(reference).bind(p.x).bind(vide(&p.produit)).bind(vide(&p.dose)).bind(categorie)
                    .execute(&mut *tx).await?;
                touches.push(r.last_insert_rowid());
            }
        }
    }
    for id in affiches.keys().filter(|id| !gardes.contains(id)) {
        sqlx::query("UPDATE acteprotocole SET actif=0 WHERE id=?").bind(id).execute(&mut *tx).await?;
        touches.push(*id);
    }
    tx.commit().await?;
    for id in touches {
        synchroniser_protocole_portees(&state.pool, id).await?;
    }
    Ok(Redirect::to("/sanitaire/plan?ok").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reperes_des_lignes() {
        let c = BandSchedule::default();
        assert_eq!(ligne_de("toutes_femelles"), Some("truies"));
        assert_eq!(ligne_de("Porcelet"), Some("porcelets"));
        assert_eq!(ligne_de("Bande"), None);
        assert_eq!(base("truies", "gestation", &c), Some(0));
        assert_eq!(base("truies", "mise_bas", &c), Some(c.gestation));
        assert_eq!(base("porcelets", "sevrage", &c), Some(c.weaning));
        assert_eq!(base("porcelets", "ia", &c), None);
    }

    #[test]
    fn la_page_se_rend() {
        let env = crate::templates::build().expect("gabarits");
        let session = json!({"csrf": "x", "peut_modifier": true, "pages": [], "identifiant": "test"});
        let plan = json!({"lignes": {"cochettes": [], "truies": [{"id": 1, "nom": "Parvo", "produit": "P", "type": "v", "dose": "", "x": 86}],
            "porcelets": [], "verrats": []}, "cycle": {"gestation": 115, "mater_avant": 5, "sevrage": 28, "transfert": 70, "depart": 180}});
        let html = env.get_template("sanitaire_plan.html").unwrap()
            .render(json!({"session": session, "plan": plan, "enregistre": true}))
            .expect("sanitaire_plan.html");
        assert!(html.contains("id=\"plan-donnees\""));
        assert!(html.contains("Parvo"));
    }
}

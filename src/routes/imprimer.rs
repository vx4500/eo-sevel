//! Page « Imprimer » : on choisit quoi imprimer (une bande, des truies, toutes
//! les truies) et quelles colonnes, puis la liste s'ouvre prête à imprimer.
//! Tout passe en GET : rien n'est modifié, et le lien se garde en favori.

use super::*;

/// (clé, libellé, expression SQL, est une date)
const COLONNES: [(&str, &str, &str, bool); 18] = [
    ("num_travail", "N° travail", "t.num_travail", false),
    ("num_national", "N° national", "t.num_national", false),
    ("rfid", "RFID", "t.rfid", false),
    ("rang", "Rang", "t.rang", false),
    ("race", "Race / lignée", "TRIM(COALESCE(t.race,'')||CASE WHEN l.nom IS NOT NULL THEN ' · '||l.nom ELSE '' END)", false),
    ("pere", "Père", "t.pere_national", false),
    ("mere", "Mère", "t.mere_national", false),
    ("naissance", "Naissance", "t.date_naissance", true),
    ("mere_cochette", "Mère à cochettes", "CASE WHEN t.mere_cochette=1 THEN 'Oui' ELSE '' END", false),
    ("bande", "Bande", "t.bande_code", false),
    ("emplacement", "Emplacement", "TRIM(COALESCE(s.nom,'')||CASE WHEN c.nom IS NOT NULL THEN ' · '||c.nom ELSE '' END||CASE WHEN COALESCE(c.num_vanne,'')<>'' THEN ' · vanne '||c.num_vanne ELSE '' END)", false),
    ("statut", "Statut", "t.statut", false),
    ("derniere_ia", "Dernière IA", "(SELECT MAX(e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type='ia')", true),
    ("echo", "Écho", "(SELECT COALESCE(e.resultat,'')||' · '||strftime('%d/%m/%Y',e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type IN('echo','echographie') ORDER BY e.date DESC,e.id DESC LIMIT 1)", false),
    ("derniere_mb", "Dernière mise bas", "(SELECT MAX(e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type='mise_bas')", true),
    ("nes_vifs", "Nés vifs (dernière MB)", "(SELECT e.nes_vifs FROM evenement e WHERE e.truie_id=t.id AND e.type='mise_bas' ORDER BY e.date DESC,e.id DESC LIMIT 1)", false),
    ("dernier_sevrage", "Dernier sevrage", "(SELECT MAX(e.date) FROM evenement e WHERE e.truie_id=t.id AND e.type='sevrage')", true),
    ("note", "Note de la fiche", "t.note", false),
];

const PAR_DEFAUT: [&str; 6] = ["num_travail", "rfid", "rang", "emplacement", "derniere_ia", "echo"];

/// Numéros de travail tapés à la main : séparés par espaces, virgules, retours.
fn numeros(texte: &str) -> Vec<String> {
    texte
        .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let bandes = generic_rows(
        &state.pool,
        "SELECT code,active FROM bande ORDER BY active DESC,date_mb DESC,id DESC",
    )
    .await?;
    let colonnes: Vec<Value> = COLONNES
        .iter()
        .map(|(cle, libelle, _, _)| json!({"cle": cle, "libelle": libelle, "coche": PAR_DEFAUT.contains(cle)}))
        .collect();
    let mut ctx = context(&session);
    ctx.insert("bandes".into(), Value::Array(bandes));
    ctx.insert("colonnes".into(), Value::Array(colonnes));
    ctx.insert("bande_choisie".into(), json!(query.get("bande")));
    ctx.insert("truies_choisies".into(), json!(query.get("truies")));
    render(&state, "imprimer.html", Value::Object(ctx))
}

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<Vec<(String, String)>>,
) -> AppResult<Html<String>> {
    let valeur = |k: &str| query.iter().find(|(c, _)| c == k).map(|(_, v)| v.trim().to_string()).filter(|v| !v.is_empty());
    let choisies: Vec<&(&str, &str, &str, bool)> = COLONNES
        .iter()
        .filter(|(cle, ..)| query.iter().any(|(k, v)| k == "col" && v == cle))
        .collect();
    if choisies.is_empty() {
        return Err(AppError::Invalid("Cochez au moins une colonne à imprimer".into()));
    }
    let portee = valeur("portee").unwrap_or_else(|| "bande".into());
    let (filtre, liens, titre): (String, Vec<String>, String) = match portee.as_str() {
        "bande" => {
            let code = valeur("bande").ok_or_else(|| AppError::Invalid("Choisissez la bande à imprimer".into()))?;
            (
                // Bande en cours : ses truies actuelles. Bande terminée (plus
                // aucune truie) : celles qui y ont eu un événement.
                "((t.reformee=0 AND t.bande_code=?1) OR (NOT EXISTS(SELECT 1 FROM truie x WHERE x.reformee=0 AND x.bande_code=?1) AND EXISTS(SELECT 1 FROM evenement e JOIN bande b ON b.id=e.bande_id WHERE e.truie_id=t.id AND b.code=?1)))".into(),
                vec![code.clone()],
                format!("Bande {code}"),
            )
        }
        "truies" => {
            let nums = numeros(&valeur("truies").unwrap_or_default());
            if nums.is_empty() {
                return Err(AppError::Invalid("Tapez le ou les numéros de travail des truies".into()));
            }
            let marques = vec!["?"; nums.len()].join(",");
            let titre = if nums.len() == 1 { format!("Truie {}", nums[0]) } else { format!("{} truies choisies", nums.len()) };
            (format!("(t.num_travail IN ({marques}) OR t.num_national IN ({marques}))"), [nums.clone(), nums].concat(), titre)
        }
        "sans_bande" => (
            "t.reformee=0 AND COALESCE(t.bande_code,'')=''".into(),
            Vec::new(),
            "Truies sans bande".into(),
        ),
        // Même définition que la page Truies improductives : actives et dans
        // aucune bande en cours.
        "improductives" => (
            "t.reformee=0 AND NOT EXISTS(SELECT 1 FROM bande b WHERE b.active=1 AND b.code=t.bande_code)".into(),
            Vec::new(),
            "Truies improductives".into(),
        ),
        _ => ("t.reformee=0".into(), Vec::new(), "Toutes les truies présentes".into()),
    };
    let ordre = match valeur("tri").as_deref() {
        Some("emplacement") => "s.nom IS NULL,s.nom,c.nom,t.num_travail",
        Some("bande") => "t.bande_code IS NULL,t.bande_code,t.num_travail",
        _ => "t.num_travail",
    };
    let select = choisies
        .iter()
        .enumerate()
        .map(|(i, (_, _, expr, _))| format!("{expr} AS c{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT {select} FROM truie t LEFT JOIN lignee_genetique l ON l.id=t.lignee_id \
         LEFT JOIN casesalle c ON c.id=t.case_id LEFT JOIN salle s ON s.id=c.salle_id \
         WHERE {filtre} ORDER BY {ordre}"
    );
    let mut requete = sqlx::query(&sql);
    for l in &liens {
        requete = requete.bind(l);
    }
    let lignes = rows_to_json(requete.fetch_all(&state.pool).await?)?;
    let cellules: Vec<Vec<Value>> = lignes
        .iter()
        .map(|ligne| {
            choisies
                .iter()
                .enumerate()
                .map(|(i, (_, _, _, date))| {
                    let v = &ligne[format!("c{i}").as_str()];
                    json!({"v": v, "date": date})
                })
                .collect()
        })
        .collect();
    let vides: i64 = valeur("vides").and_then(|v| v.parse().ok()).filter(|n| (0..=6).contains(n)).unwrap_or(0);
    let mut ctx = context(&session);
    ctx.insert("titre".into(), json!(titre));
    ctx.insert("entetes".into(), json!(choisies.iter().map(|(_, l, _, _)| *l).collect::<Vec<_>>()));
    ctx.insert("vides".into(), json!(vides));
    ctx.insert("lignes".into(), json!(cellules));
    ctx.insert("retour".into(), json!(format!("/imprimer?{}", query.iter().map(|(k, v)| format!("{}={}", urlencoding_simple(k), urlencoding_simple(v))).collect::<Vec<_>>().join("&"))));
    render(&state, "imprimer_liste.html", Value::Object(ctx))
}

fn urlencoding_simple(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeros_separes_par_espaces_virgules_retours() {
        assert_eq!(numeros("409298, 409357\n500419 ;501336"), vec!["409298", "409357", "500419", "501336"]);
        assert!(numeros("  ").is_empty());
    }

    #[test]
    fn colonnes_par_defaut_existent() {
        for cle in PAR_DEFAUT {
            assert!(COLONNES.iter().any(|(c, ..)| *c == cle), "{cle}");
        }
    }
}

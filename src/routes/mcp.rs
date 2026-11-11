//! Connecteur Claude (MCP, transport HTTP « streamable », réponses JSON).
//!
//! L'application Claude du téléphone parle avec l'éleveur — à la voix — et
//! appelle ces outils pour lire ou enregistrer. Adresse : `/mcp/{jeton}`, le
//! jeton (paramètre `mcp_jeton`) tient lieu de mot de passe ; il se crée et
//! se change dans Mes équipements → Assistant.
//!
//! Toute écriture se fait en deux temps : sans `confirme`, l'outil ne fait
//! que décrire ce qu'il va faire ; Claude le relit à l'éleveur et ne rappelle
//! avec `confirme: true` qu'après son accord. Chaque écriture est tracée dans
//! `journal`.

use super::*;
use crate::assistant;
use axum::Json;
use chrono::Datelike;

const PROTOCOLES: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const AUTEUR: &str = "Claude (connecteur)";

fn outils() -> Value {
    let confirme = json!({"type": "boolean", "description": "false ou absent : décrit seulement l'action. true : enregistre — uniquement après accord explicite de l'éleveur."});
    json!([
        {"name": "chercher_truie",
         "description": "Fiche d'une truie par son numéro de travail (ou fin de numéro) : statut, rang, bande, salle, case/vanne et derniers événements.",
         "input_schema": {"type": "object", "properties": {"numero": {"type": "string"}}, "required": ["numero"]}},
        {"name": "cases_salle",
         "description": "Cases d'une salle avec numéro de vanne, places, truies présentes et nombre de porcs. La salle s'écrit comme on la dit : « Verraterie », « Engr2 », « eng 3 », « PS1 », « Mater 2 ».",
         "input_schema": {"type": "object", "properties": {"salle": {"type": "string", "description": "nom complet ou abrégé de la salle"}}, "required": ["salle"]}},
        {"name": "etat_salle",
         "description": "État d'une salle en une fois : température, hygrométrie, consigne, ventilation réelle, forçage en cours, nombre de porcs ou de truies, bande et jour de présence, alertes. Ex. « comment va l'Engr2 ? », « combien de porcs en PS1 ? ». Sans salle : toutes les salles.",
         "input_schema": {"type": "object", "properties": {"salle": {"type": "string", "description": "nom complet ou abrégé (Engr2, PS1, Mater 1, verraterie) ; vide = toutes"}}}},
        {"name": "forcer_ventilation",
         "description": "Force la ventilation d'une salle à un pourcentage pendant une durée, puis la régulation (ou l'ancien réglage du variateur) revient toute seule. Ex. « fais tourner l'Engr2 à 100 % pendant 2 h », « PS1 à 50 % pendant 30 minutes ». Salle en nom complet ou abrégé. Durée 120 min par défaut, 24 h au plus. Deux temps : d'abord sans confirme, lire le résumé à l'éleveur, puis confirme=true après son accord.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string", "description": "nom complet ou abrégé de la salle"},
             "pourcentage": {"type": "number", "description": "0 à 100"},
             "duree_minutes": {"type": "integer", "description": "durée en minutes, 120 par défaut, 1440 au plus"},
             "confirme": confirme}, "required": ["salle", "pourcentage"]}},
        {"name": "arreter_forcage",
         "description": "Arrête tout de suite un forçage de ventilation en cours (ex. « remets l'Engr2 en automatique ») : la régulation reprend, ou le variateur retrouve sa valeur d'avant. Deux temps comme forcer_ventilation.",
         "input_schema": {"type": "object", "properties": {"salle": {"type": "string"}, "confirme": confirme}, "required": ["salle"]}},
        {"name": "regler_ambiance",
         "description": "Change durablement un réglage de régulation d'une salle : consigne de température fixe (t_cible), plage (°C entre ventilation mini et maxi), ventilation minimale ou maximale (%). Ex. « mets la consigne de la gestante à 20 », « ventilation mini de PS1 à 25 % ». Pour un besoin passager, préférer forcer_ventilation. Deux temps comme forcer_ventilation.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string"},
             "t_cible": {"type": "number", "description": "consigne fixe en °C (ignorée si la salle suit une courbe)"},
             "plage": {"type": "number", "description": "°C"},
             "ventil_min": {"type": "number", "description": "%"},
             "ventil_max": {"type": "number", "description": "%"},
             "confirme": confirme}, "required": ["salle"]}},
        {"name": "que_sais_tu_faire",
         "description": "Liste en français ce que le connecteur sait faire, par famille (animaux, mouvements, ambiance, économie, mémoire, questions libres), et ce qu'il ne fait pas encore.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "calendrier_ia",
         "description": "Dates d'IA prévues par bande active : IA de la bande (mise-bas − gestation) et IA des truies sevrées (lundi suivant le sevrage). Sert à calculer un rappel « X jours avant les IA ».",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "deplacer_truie",
         "description": "Place une truie dans une case. La case se désigne par la salle et le numéro de vanne ou le nom de case. Deux temps : d'abord sans confirme, lire le résumé à l'éleveur, puis confirme=true après son accord.",
         "input_schema": {"type": "object", "properties": {
             "numero": {"type": "string"}, "salle": {"type": "string"},
             "vanne": {"type": "string", "description": "numéro de vanne ou nom de la case"},
             "confirme": confirme}, "required": ["numero", "salle", "vanne"]}},
        {"name": "creer_rappel",
         "description": "Crée un rappel (tâche avec échéance) affiché dans EO_Sevel. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "titre": {"type": "string", "description": "ex. « Truie 400042 : mettre sous œstrogène »"},
             "date": {"type": "string", "description": "AAAA-MM-JJ"},
             "note": {"type": "string"}, "bande": {"type": "string"},
             "confirme": confirme}, "required": ["titre", "date"]}},
        {"name": "declarer_mort",
         "description": "Déclare des porcs morts dans une salle d'engraissement ou de post-sevrage (pas la maternité : les porcelets se saisissent par portée). Ex. « un mort en Engr3, respiratoire ». Salle en nom complet ou abrégé ; case facultative si la salle n'en a qu'une ou si la plus pleine suffit. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string", "description": "nom complet ou abrégé"}, "case": {"type": "string", "description": "numéro de vanne ou nom de la case (facultatif)"},
             "nombre": {"type": "integer"}, "cause": {"type": "string", "description": "une cause configurée (la liste est renvoyée si elle est inconnue)"},
             "stade": {"type": "string", "description": "facultatif : déduit de la salle"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "nombre", "cause"]}},
        {"name": "transferer_porcs",
         "description": "Transfère des porcs (porcelets, charcutiers) d'une salle vers une autre ; ils gardent leur bande. Ex. « passe 20 porcs de PS1 en Engr2 », « transfère 68 porcs de l'engraissement 2 vers l'engraissement 3 ». Salles en nom complet ou abrégé (Engr2, eng 3, PS1). Pris dans les cases les plus pleines ; la case d'arrivée n'est demandée que si la salle en a plusieurs. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle_source": {"type": "string", "description": "nom complet ou abrégé"}, "case_source": {"type": "string", "description": "facultatif"},
             "salle_destination": {"type": "string", "description": "nom complet ou abrégé"}, "case_destination": {"type": "string", "description": "facultatif si la salle n'a qu'une case"},
             "nombre": {"type": "integer"}, "bande": {"type": "string", "description": "facultatif : vérifie que les porcs sont de cette bande"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle_source", "salle_destination", "nombre"]}},
        {"name": "sevrer_salle",
         "description": "Sevre toute une salle de maternité d'un coup (ex. « sèvre la Mater 1, porcelets en PS1, truies en verraterie ») : chaque portée ouverte de la salle est sevrée avec ses porcelets présents, les porcelets vont dans la case de post-sevrage indiquée et les truies dans la salle indiquée (verraterie par défaut, sans case précise). Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string", "description": "salle de maternité, ex. « Mater 1 »"},
             "destination": {"type": "string", "description": "case(s) de post-sevrage pour les porcelets, ex. « PS1 » ou « PS1, PS2 » : les portées entières remplissent la première, puis la suivante"},
             "salle_truies": {"type": "string", "description": "salle où vont les truies, « Verraterie » par défaut"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "destination"]}},
        {"name": "diagnostiquer_portees",
         "description": "À lancer D'ABORD quand un sevrage est refusé ou qu'un effectif de porcelets paraît faux : pour chaque portée ouverte d'une salle de maternité (ou d'une truie), nés totaux et vivants, morts-nés, adoptés, retirés, pertes, présents calculés, et les anomalies en français avec la correction proposée (truie réformée avec portée ouverte → cloturer_portee ; présents négatifs → corriger_mise_bas ; pertes datées avant la mise-bas → corriger_perte ; retirés au-delà des nés vivants ; sevrage sans nombre de sevrés). Lecture seule : propose à l'éleveur, puis applique chaque correction avec l'outil indiqué après son accord.",
         "input_schema": {"type": "object", "properties": {"salle": {"type": "string", "description": "maternité, ex. « Mater 2 »"}, "truie": {"type": "string", "description": "ou un numéro de truie"}}}},
        {"name": "cloturer_portee",
         "description": "Clôture la portée ouverte d'une truie sans passer par le sevrage de bande (truie réformée ou partie, porcelets déjà comptés ailleurs) : un événement sevrage à nb_sevres (0 par défaut). Ex. après diagnostiquer_portees : « clôture la portée de 500371 ». Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "truie": {"type": "string"}, "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "nb_sevres": {"type": "integer", "description": "0 par défaut"}, "note": {"type": "string", "description": "raison"},
             "confirme": confirme}, "required": ["truie"]}},
        {"name": "corriger_mise_bas",
         "description": "Corrige les chiffres de la dernière mise-bas d'une truie : nés vivants, nés totaux, morts-nés, éventuellement la date. La note (raison) est obligatoire et s'ajoute à la mise-bas. Ex. « +2 nés vivants sur 500371, pertes saisies avant la mise-bas ». Le résumé donne avant/après. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "truie": {"type": "string"},
             "nes_vifs": {"type": "integer"}, "nes_totaux": {"type": "integer"}, "mort_nes": {"type": "integer"},
             "date": {"type": "string", "description": "nouvelle date AAAA-MM-JJ (facultatif)"},
             "note": {"type": "string", "description": "raison, obligatoire"},
             "confirme": confirme}, "required": ["truie", "note"]}},
        {"name": "corriger_perte",
         "description": "Change la date d'une perte de porcelets (numéro donné par diagnostiquer_portees) ou la supprime. Ex. « mets la perte 62 au 14/10 ». Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "id": {"type": "integer"}, "date": {"type": "string", "description": "nouvelle date AAAA-MM-JJ"},
             "supprimer": {"type": "boolean"}, "note": {"type": "string", "description": "raison"},
             "confirme": confirme}, "required": ["id"]}},
        {"name": "inventaire_case",
         "description": "Enregistre un comptage réel de porcs dans une case (inventaire) : à partir de là, EO_Sevel repart de ce nombre. Ex. « il y a 100 porcelets en PS2 ». Salle en nom abrégé, case facultative si la salle n'en a qu'une. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string"}, "case": {"type": "string"}, "nombre": {"type": "integer"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"}, "note": {"type": "string"},
             "confirme": confirme}, "required": ["salle", "nombre"]}},
        {"name": "enregistrer_achat",
         "description": "Enregistre un achat (facture) dans l'économie, comme la saisie à la main de la page Économie : semence, vétérinaire, génétique ou aliment. Ex. « ajoute 510 € de semence LB-CIA à la B3.14 ». Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "categorie": {"type": "string", "enum": ["semence", "veto", "genetique", "aliment"]},
             "montant_ht": {"type": "number", "description": "montant hors taxes en euros"},
             "fournisseur": {"type": "string"},
             "designation": {"type": "string", "description": "produit ou désignation de la facture"},
             "quantite": {"type": "number", "description": "doses (semence), quantité (veto), animaux (génétique) ou tonnes (aliment)"},
             "bande": {"type": "string", "description": "code de la bande concernée, ex. « B3.14 » (facultatif)"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, date de la facture ; aujourd'hui par défaut"},
             "num_facture": {"type": "string"}, "note": {"type": "string"},
             "confirme": confirme}, "required": ["categorie", "montant_ht"]}},
        {"name": "depart_porcs",
         "description": "Enregistre un départ de porcs à l'abattoir depuis une salle (pris dans les cases les plus pleines). Ex. « 30 porcs partis de l'Engr1 ce matin ». Salle en nom complet ou abrégé. Deux temps comme deplacer_truie.",
         "input_schema": {"type": "object", "properties": {
             "salle": {"type": "string"}, "case": {"type": "string", "description": "facultatif"},
             "nombre": {"type": "integer"},
             "date": {"type": "string", "description": "AAAA-MM-JJ, aujourd'hui par défaut"},
             "confirme": confirme}, "required": ["salle", "nombre"]}},
        {"name": "rappels",
         "description": "Rappels et tâches non faits, par échéance.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "memoire",
         "description": "Ce que l'éleveur a demandé de retenir (habitudes, vocabulaire, consignes). Déjà fournies dans les consignes ; sert à retrouver un numéro pour oublier.",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "retenir",
         "description": "Retient une information pour les prochaines conversations (ex. « la vanne 12 de la Verraterie est en panne », « dire IA pour insémination »). À utiliser quand l'éleveur dit « retiens », « souviens-toi », ou explique une habitude durable.",
         "input_schema": {"type": "object", "properties": {"texte": {"type": "string"}}, "required": ["texte"]}},
        {"name": "oublier",
         "description": "Supprime une information retenue (numéro donné par memoire).",
         "input_schema": {"type": "object", "properties": {"numero": {"type": "integer"}}, "required": ["numero"]}},
        {"name": "lister_tables",
         "description": "Pour les autres questions : tables de la base de l'élevage (lecture seule).",
         "input_schema": {"type": "object", "properties": {}}},
        {"name": "decrire_tables",
         "description": "Schéma des tables demandées.",
         "input_schema": {"type": "object", "properties": {"tables": {"type": "array", "items": {"type": "string"}}}, "required": ["tables"]}},
        {"name": "requete",
         "description": "UNE requête SQLite SELECT/WITH en lecture seule, 200 lignes au plus.",
         "input_schema": {"type": "object", "properties": {"sql": {"type": "string"}}, "required": ["sql"]}}
    ])
}

const CONSIGNES: &str = "Outils de l'élevage EO_Sevel. Réponds brièvement, en français parlé (l'éleveur est souvent à la voix, dans le bâtiment). \
Avant toute écriture, appelle l'outil sans confirme, répète en une phrase ce qui va être fait et attends un « oui » ; seulement ensuite rappelle avec confirme=true. \
N'invente jamais un numéro, une date ou une case : vérifie avec les outils. Les salles se disent en abrégé (Engr2, PS1, Mater 1) : passe le nom tel quel, les outils le reconnaissent. Pour « mets la salle à X % », utilise forcer_ventilation (retour automatique) ; pour l'état d'une salle, etat_salle. \
Quand une action est refusée, cherche la cause (diagnostiquer_portees pour un sevrage ou un effectif), explique-la, propose la correction, et ne l'applique qu'après accord. Pour une date relative aux IA, utilise calendrier_ia. \
Ce qui a été retenu est listé ci-dessous (pas besoin d'appeler memoire). Quand l'éleveur explique quelque chose de durable, retiens-le en une phrase courte (outil retenir).";

async fn table_memoire(pool: &SqlitePool) -> Result<(), String> {
    sqlx::query("CREATE TABLE IF NOT EXISTS assistant_memoire(id INTEGER PRIMARY KEY, texte TEXT NOT NULL, cree_le TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP)")
        .execute(pool).await.map(|_| ()).map_err(|e| e.to_string())
}

async fn notes_memoire(pool: &SqlitePool) -> Result<Vec<(i64, String, String)>, String> {
    table_memoire(pool).await?;
    sqlx::query_as("SELECT id,texte,substr(cree_le,1,10) FROM assistant_memoire ORDER BY id")
        .fetch_all(pool).await.map_err(|e| e.to_string())
}

fn notes_en_texte(notes: &[(i64, String, String)]) -> String {
    notes.iter().map(|(i, t, d)| format!("{i}. {t} (le {d})")).collect::<Vec<_>>().join("\n")
}

fn texte(t: impl Into<String>, erreur: bool) -> Value {
    json!({"content": [{"type": "text", "text": t.into()}], "isError": erreur})
}

fn arg<'a>(a: &'a Value, k: &str) -> &'a str {
    a.get(k).and_then(Value::as_str).map(str::trim).unwrap_or("")
}

async fn journaliser(pool: &SqlitePool, action: &str, objet: &str, detail: &str) {
    let _ = sqlx::query("INSERT INTO journal(horodatage,utilisateur,action,objet,detail,chemin) VALUES(CURRENT_TIMESTAMP,?,?,?,?,'/mcp')")
        .bind(AUTEUR).bind(action).bind(objet).bind(detail)
        .execute(pool).await;
}

async fn truie_par_numero(pool: &SqlitePool, numero: &str) -> Result<(i64, String), String> {
    let n = numero.replace([' ', '.'], "");
    let lignes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id,num_travail FROM truie WHERE reformee=0 AND (num_travail=?1 OR replace(num_travail,' ','')=?1 OR num_national=?1 OR num_travail LIKE '%'||?1) ORDER BY num_travail=?1 DESC LIMIT 5",
    )
    .bind(&n)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    match lignes.as_slice() {
        [] => Err(format!("Aucune truie présente ne porte le numéro {numero}.")),
        [une] => Ok(une.clone()),
        [une, ..] if une.1 == n => Ok(une.clone()),
        plusieurs => Err(format!("Plusieurs truies correspondent : {}. Lequel ?", plusieurs.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

/// Comme `truie_par_numero`, mais les truies réformées comptent aussi : une
/// portée ouverte peut rester sur une truie partie.
async fn truie_par_numero_toute(pool: &SqlitePool, numero: &str) -> Result<(i64, String), String> {
    let n = numero.replace([' ', '.'], "");
    if n.is_empty() { return Err("Quel numéro de truie ?".into()); }
    let lignes: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id,num_travail FROM truie WHERE num_travail=?1 OR replace(num_travail,' ','')=?1 OR num_national=?1 OR num_travail LIKE '%'||?1 ORDER BY num_travail=?1 DESC, reformee, id DESC LIMIT 5",
    ).bind(&n).fetch_all(pool).await.map_err(|e| e.to_string())?;
    match lignes.as_slice() {
        [] => Err(format!("Aucune truie ne porte le numéro {numero}.")),
        [une] => Ok(une.clone()),
        [une, ..] if une.1 == n => Ok(une.clone()),
        plusieurs => Err(format!("Plusieurs truies correspondent : {}. Laquelle ?", plusieurs.iter().map(|l| l.1.as_str()).collect::<Vec<_>>().join(", "))),
    }
}

async fn case_par_vanne(pool: &SqlitePool, salle: &str, vanne: &str) -> Result<(i64, i64, String, String, Option<i64>), String> {
    let v = vanne.trim_start_matches(['v', 'V']).trim().to_string();
    let (salle_id, _, _) = salle_par_nom(pool, salle).await?;
    let lignes: Vec<(i64, i64, String, String, Option<i64>)> = sqlx::query_as(
        "SELECT c.id,c.salle_id,s.nom,c.nom,COALESCE(c.nb_max_truies,c.nb_max_porcs) FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         WHERE c.salle_id=?1 AND (c.num_vanne=?2 OR lower(c.nom)=lower(?2) OR c.nom LIKE ?2||' %')",
    )
    .bind(salle_id).bind(&v)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    match lignes.len() {
        0 => Err(format!("Pas de case « {vanne} » dans la salle « {salle} ». Utilise cases_salle pour voir les cases.")),
        1 => Ok(lignes.into_iter().next().unwrap()),
        _ => Err(format!("Plusieurs cases correspondent : {}.", lignes.iter().map(|l| format!("{} / {}", l.2, l.3)).collect::<Vec<_>>().join(", "))),
    }
}

/// Clé de comparaison d'un nom de salle tel qu'on le dit : minuscules, sans
/// accent ni espace, préfixes ramenés à une forme courte. « Engraissement 2 »,
/// « Engr2 », « eng 2 » et « E2 » donnent tous « eng2 » ; « Post-sevrage 1 »
/// et « PS1 » donnent « ps1 » ; « Mater 1 » et « maternité 1 » donnent « mater1 ».
pub(super) fn cle_salle(nom: &str) -> String {
    let mut cle: String = nom.to_lowercase().chars().filter_map(|c| match c {
        'é' | 'è' | 'ê' | 'ë' => Some('e'), 'à' | 'â' => Some('a'), 'î' | 'ï' => Some('i'),
        'ô' => Some('o'), 'ù' | 'û' => Some('u'), 'ç' => Some('c'),
        c if c.is_alphanumeric() => Some(c), _ => None,
    }).collect();
    for (long, court) in [("postsevrage", "ps"), ("postengraissement", "posteng"), ("engraissement", "eng"), ("engr", "eng"),
                          ("maternite", "mater"), ("verraterie", "verrat"), ("gestantes", "gest"), ("gestante", "gest"), ("gestation", "gest")] {
        if cle.starts_with(long) { cle = format!("{court}{}", &cle[long.len()..]); break; }
    }
    // « mat1 » → « mater1 », « e2 » → « eng2 », « p1 » → « ps1 » : ce qu'on dit vite.
    if let Some(reste) = cle.strip_prefix("mat").filter(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit())) { cle = format!("mater{reste}"); }
    if let Some(reste) = cle.strip_prefix('e').filter(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit())) { cle = format!("eng{reste}"); }
    if let Some(reste) = cle.strip_prefix('p').filter(|r| r.chars().next().is_some_and(|c| c.is_ascii_digit())) { cle = format!("ps{reste}"); }
    cle
}

/// Salle désignée comme on la dit : (id, nom, type). Un nom exact gagne ;
/// sinon un seul nom qui commence par ou contient la clé ; sinon la liste
/// des salles possibles, pour que l'éleveur précise.
pub(super) async fn salle_par_nom(pool: &SqlitePool, texte: &str) -> Result<(i64, String, Option<String>), String> {
    let cle = cle_salle(texte);
    if cle.is_empty() { return Err("Quelle salle ?".into()); }
    let salles: Vec<(i64, String, Option<String>)> = sqlx::query_as("SELECT id,nom,type FROM salle ORDER BY ordre,nom")
        .fetch_all(pool).await.map_err(|e| e.to_string())?;
    if let Some(s) = salles.iter().find(|s| cle_salle(&s.1) == cle) { return Ok(s.clone()); }
    for test in [|c: &str, k: &str| c.starts_with(k), |c: &str, k: &str| c.contains(k)] {
        let trouvees: Vec<&(i64, String, Option<String>)> = salles.iter().filter(|s| test(&cle_salle(&s.1), &cle)).collect();
        match trouvees.as_slice() {
            [une] => return Ok((*une).clone()),
            [] => {}
            plusieurs => return Err(format!("Plusieurs salles correspondent à « {texte} » : {}. Laquelle ?",
                plusieurs.iter().map(|s| s.1.as_str()).collect::<Vec<_>>().join(", "))),
        }
    }
    Err(format!("Pas de salle « {texte} ». Salles : {}.", salles.iter().map(|s| s.1.as_str()).collect::<Vec<_>>().join(", ")))
}

/// Porcs présents par case d'une salle (case facultative), la plus pleine
/// d'abord : (case_id, salle_id, salle, case, effectif). La maternité est
/// refusée : ses porcelets se comptent par portée, saisie dans EO_Sevel.
async fn cases_porcs(pool: &SqlitePool, salle: &str, case: &str) -> Result<Vec<(i64, i64, String, String, i64)>, String> {
    let (salle_id, nom, type_salle) = salle_par_nom(pool, salle).await?;
    if stade_pour_type_salle(type_salle.as_deref().unwrap_or("")).as_deref() == Some("Maternité") {
        return Err(format!("{nom} est une maternité : les porcelets se saisissent par portée dans EO_Sevel."));
    }
    let lignes: Vec<(i64, i64, String, String)> = sqlx::query_as(
        "SELECT c.id,c.salle_id,s.nom,c.nom FROM casesalle c JOIN salle s ON s.id=c.salle_id \
         WHERE c.salle_id=?1 AND (?2='' OR c.num_vanne=?2 OR lower(c.nom)=lower(?2) OR c.nom LIKE ?2||' %' OR replace(lower(c.nom),' ','')=replace(lower(?2),' ','')) ORDER BY c.id",
    )
    .bind(salle_id).bind(case.trim_start_matches(['v', 'V']).trim())
    .fetch_all(pool).await.map_err(|e| e.to_string())?;
    if lignes.is_empty() {
        let toutes: Vec<String> = sqlx::query_scalar("SELECT nom FROM casesalle WHERE salle_id=? ORDER BY id").bind(salle_id)
            .fetch_all(pool).await.unwrap_or_default();
        return Err(if toutes.is_empty() { format!("{nom} n'a aucune case : à créer dans Mes salles.") }
            else { format!("Pas de case « {case} » en {nom}. Cases : {}.", toutes.join(", ")) });
    }
    let mut sortie = Vec::new();
    for (id, salle_id, s, c) in lignes {
        let n = case_pig_count(pool, id).await.map_err(|e| e.to_string())?;
        sortie.push((id, salle_id, s, c, n));
    }
    sortie.sort_by(|a, b| b.4.cmp(&a.4));
    Ok(sortie)
}

/// Répartit `nombre` porcs sur les cases (les plus pleines d'abord).
fn prelever(cases: &[(i64, i64, String, String, i64)], nombre: i64) -> Result<Vec<(i64, i64, i64)>, String> {
    let present: i64 = cases.iter().map(|c| c.4).sum();
    if nombre <= 0 {
        return Err("Le nombre doit être d'au moins 1.".into());
    }
    if nombre > present {
        return Err(format!("Il n'y a que {present} porc(s) en {}{} ({nombre} demandés). Redemande avec {present} au plus, ou corrige l'inventaire dans EO_Sevel.",
            cases[0].2, if cases.len() > 1 { format!(" : {}", cases.iter().map(|c| format!("{} {}", c.3, c.4)).collect::<Vec<_>>().join(", ")) } else { String::new() }));
    }
    let mut reste = nombre;
    let mut pris = Vec::new();
    for (case_id, salle_id, _, _, n) in cases {
        if reste == 0 { break; }
        let p = reste.min(*n);
        if p > 0 { pris.push((*case_id, *salle_id, p)); reste -= p; }
    }
    Ok(pris)
}

fn date_ou_aujourdhui(a: &Value) -> Result<String, String> {
    let auj = chrono::Local::now().date_naive();
    let d = match arg(a, "date") {
        "" => auj,
        t => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?,
    };
    if d > auj { return Err("Date dans le futur.".into()); }
    Ok(d.to_string())
}

async fn code_bande(pool: &SqlitePool, bande_id: Option<i64>) -> Option<String> {
    sqlx::query_scalar("SELECT code FROM bande WHERE id=?").bind(bande_id?).fetch_optional(pool).await.ok().flatten()
}

fn confirme(a: &Value) -> bool {
    a.get("confirme").and_then(Value::as_bool) == Some(true)
}

/// Réglages d'ambiance d'une salle utiles au forçage : (actif, pilote,
/// ipx_hote, sortie_ventil, forcage_pct, forcage_fin, forcage_retour).
type LigneAmbiance = (i64, i64, Option<String>, Option<String>, Option<f64>, Option<String>, Option<f64>);

async fn ambiance_de(pool: &SqlitePool, salle_id: i64) -> Result<Option<LigneAmbiance>, String> {
    sqlx::query_as("SELECT actif,pilote,ipx_hote,sortie_ventil,forcage_pct,forcage_fin,forcage_retour FROM ambiance_salle WHERE salle_id=?")
        .bind(salle_id).fetch_optional(pool).await.map_err(|e| e.to_string())
}

fn l_forcage_actif(l: Option<&LigneAmbiance>) -> bool {
    l.is_some_and(|l| crate::regul_core::forcage_en_cours(l.4, l.5.as_deref(), &crate::regul_core::instant_utc(chrono::Utc::now().timestamp())).is_some())
}

/// Variateur Zigbee rangé pour la ventilation de la salle (table ipx_point,
/// rôle ventilation, hôte zigbee) : chemin « <ieee>/brightness ».
async fn variateur_zigbee(pool: &SqlitePool, salle_id: i64) -> Result<Option<String>, String> {
    sqlx::query_scalar(
        "SELECT COALESCE(NULLIF(chemin_ecriture,''),chemin) FROM ipx_point WHERE salle_id=? AND role='ventilation' AND hote='zigbee' AND actif=1 ORDER BY ordre,id LIMIT 1",
    ).bind(salle_id).fetch_optional(pool).await.map_err(|e| e.to_string())
}

/// Pourcentage réel d'un variateur Zigbee d'après la dernière valeur reçue.
async fn pourcent_variateur(pool: &SqlitePool, chemin: &str) -> Option<f64> {
    let ieee = chemin.trim().strip_suffix("/brightness")?;
    let brightness: Option<f64> = sqlx::query_scalar("SELECT valeur FROM zigbee_valeur WHERE ieee=? AND cle='brightness'")
        .bind(ieee).fetch_optional(pool).await.ok().flatten().flatten();
    let etat: Option<f64> = sqlx::query_scalar("SELECT valeur FROM zigbee_valeur WHERE ieee=? AND cle='state'")
        .bind(ieee).fetch_optional(pool).await.ok().flatten().flatten();
    Some(if etat == Some(0.0) { 0.0 } else { crate::regul_core::variateur_vers_pourcent(brightness?) })
}

/// Dépose une consigne pour un variateur Zigbee (publiée dans la seconde par
/// le pont d'eo-regul).
async fn commander_variateur(pool: &SqlitePool, chemin: &str, pct: f64) -> Result<(), String> {
    let (ieee, cle) = chemin.trim().split_once('/').ok_or("chemin Zigbee invalide")?;
    let valeur = if cle == "brightness" { crate::regul_core::pourcent_vers_variateur(pct) } else { pct };
    let charge = match cle {
        "brightness" => json!({"brightness": valeur.round().clamp(0.0, 255.0) as i64}),
        "state" => json!({"state": if valeur >= 0.5 { "ON" } else { "OFF" }}),
        autre => json!({autre: valeur}),
    };
    sqlx::query("INSERT INTO zigbee_commande(ieee,charge,demande) VALUES(?,?,?)")
        .bind(ieee).bind(charge.to_string()).bind(chrono::Utc::now().timestamp())
        .execute(pool).await.map(|_| ()).map_err(|e| e.to_string())
}

/// Le démon eo-regul donne-t-il signe de vie ? Sans lui, un forçage ne
/// finirait jamais : on refuse plutôt que de laisser une salle à 100 %.
async fn demon_vivant(pool: &SqlitePool) -> bool {
    super::ambiance::etat_demon(pool).await["actif"].as_bool() == Some(true)
}

fn heure_locale(fin_utc: &str) -> String {
    chrono::NaiveDateTime::parse_from_str(fin_utc.trim(), "%Y-%m-%d %H:%M:%S").ok()
        .map(|d| d.and_utc().with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_else(|| fin_utc.to_string())
}

fn duree_texte(minutes: i64) -> String {
    match (minutes / 60, minutes % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m:02}"),
    }
}

/// État complet d'une salle pour `etat_salle`.
async fn etat_salle(pool: &SqlitePool, salle_id: i64, nom: &str, type_salle: Option<&str>) -> Value {
    let etat: Option<(i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>, Option<i64>, Option<String>, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT e.ts,e.t,e.h,e.consigne,e.ventil,e.jour_bande,e.mode,e.sonde_ok,e.message,b.code FROM ambiance_etat e LEFT JOIN bande b ON b.id=e.bande_id WHERE e.salle_id=?",
    ).bind(salle_id).fetch_optional(pool).await.ok().flatten();
    let ambiance = ambiance_de(pool, salle_id).await.ok().flatten();
    let instant = chrono::Utc::now().timestamp();
    let forcage = ambiance.as_ref().and_then(|a| super::ambiance::forcage_affiche(a.4, a.5.as_deref(), instant));
    let regulee = ambiance.as_ref().is_some_and(|a| a.0 != 0);
    let variateur = variateur_zigbee(pool, salle_id).await.ok().flatten();
    let ventil_reelle = match (&etat, &variateur) {
        (Some(e), _) if regulee => e.4,
        (_, Some(chemin)) => pourcent_variateur(pool, chemin).await,
        _ => None,
    };
    let cases: Vec<(i64, String)> = sqlx::query_as("SELECT id,nom FROM casesalle WHERE salle_id=? ORDER BY id").bind(salle_id).fetch_all(pool).await.unwrap_or_default();
    let mut porcs = 0i64;
    let mut detail = Vec::new();
    let mut bandes = std::collections::BTreeSet::new();
    let auj = chrono::Local::now().date_naive().to_string();
    for (id, case) in &cases {
        let n = case_pig_count(pool, *id).await.unwrap_or(0);
        porcs += n;
        if let Some(code) = code_bande(pool, bande_de_la_case(pool, *id, &auj).await.ok().flatten()).await { if n > 0 { bandes.insert(code); } }
        if cases.len() > 1 && n > 0 { detail.push(format!("{case} : {n}")); }
    }
    let truies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie t WHERE t.reformee=0 AND (t.salle_id=?1 OR t.case_id IN (SELECT id FROM casesalle WHERE salle_id=?1))")
        .bind(salle_id).fetch_one(pool).await.unwrap_or(0);
    let alertes: Vec<String> = sqlx::query_scalar("SELECT message FROM ambiance_alerte WHERE salle_id=? AND fin IS NULL ORDER BY debut")
        .bind(salle_id).fetch_all(pool).await.unwrap_or_default();
    let age_min = etat.as_ref().map(|e| (instant - e.0) / 60);
    json!({
        "salle": nom, "type": type_salle,
        "regulation": if !regulee { "aucune (variateur réglé à la main)" } else if ambiance.as_ref().is_some_and(|a| a.1 != 0) { "pilotée par EO_Sevel" } else { "observation seulement" },
        "temperature": etat.as_ref().and_then(|e| e.1), "hygrometrie": etat.as_ref().and_then(|e| e.2),
        "consigne": etat.as_ref().and_then(|e| e.3), "ventilation_pct": ventil_reelle,
        "mode": etat.as_ref().and_then(|e| e.6.clone()), "sonde_ok": etat.as_ref().map(|e| e.7 != 0),
        "mesure_il_y_a_min": age_min, "message": etat.as_ref().and_then(|e| e.8.clone()),
        "forcage": forcage.map(|f| format!("forcé à {} % jusqu'à {}", f["pct"], f["fin"].as_str().unwrap_or(""))),
        "porcs": porcs, "porcs_par_case": detail, "truies": truies,
        "bande": if bandes.is_empty() { etat.as_ref().and_then(|e| e.9.clone()) } else { Some(bandes.into_iter().collect::<Vec<_>>().join(", ")) },
        "jour_de_presence": etat.as_ref().and_then(|e| e.5),
        "alertes": alertes,
    })
}

const FAMILLES: &str = "\
Truies : chercher_truie, deplacer_truie (vers une case), sevrer_salle (toute une maternité).\n\
Porcs : transferer_porcs (salle à salle, noms abrégés acceptés), declarer_mort, depart_porcs (abattoir), cases_salle, inventaire_case (comptage réel).\n\
Portées : diagnostiquer_portees (anomalies et corrections proposées), cloturer_portee, corriger_mise_bas, corriger_perte.\n\
Ambiance : etat_salle (température, ventilation, porcs, bande), forcer_ventilation (X % pendant N min, retour automatique), arreter_forcage, regler_ambiance (consigne, plage, ventil mini/maxi).\n\
Économie : enregistrer_achat (semence, véto, génétique, aliment).\n\
Organisation : rappels, creer_rappel, calendrier_ia.\n\
Mémoire : memoire, retenir, oublier.\n\
Questions libres (lecture seule) : lister_tables, decrire_tables, requete.\n\
Pas encore : saisie des IA, échos et mises-bas, soins et vaccins, pesées, relevés de compteurs et de silos, éclairage, commandes Zigbee directes. Jamais : code, mises à jour, sauvegardes, utilisateurs, relais de l'automate.";

async fn appeler(state: &AppState, nom: &str, a: &Value) -> Result<String, String> {
    let pool = &state.pool;
    match nom {
        "chercher_truie" => {
            let (id, _) = truie_par_numero(pool, arg(a, "numero")).await?;
            let fiche: Option<(String, String, i64, Option<String>, Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT t.num_travail,t.statut,t.rang,t.bande_code,s.nom,c.nom,c.num_vanne FROM truie t \
                 LEFT JOIN salle s ON s.id=t.salle_id LEFT JOIN casesalle c ON c.id=t.case_id WHERE t.id=?",
            ).bind(id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (num, statut, rang, bande, salle, case, vanne) = fiche.ok_or("Truie introuvable")?;
            let evts: Vec<(String, String)> = sqlx::query_as("SELECT date,type FROM evenement WHERE truie_id=? ORDER BY date DESC LIMIT 6")
                .bind(id).fetch_all(pool).await.unwrap_or_default();
            Ok(json!({"numero": num, "statut": statut, "rang": rang, "bande": bande, "salle": salle, "case": case, "vanne": vanne,
                      "derniers_evenements": evts.into_iter().map(|(d, t)| format!("{d} {t}")).collect::<Vec<_>>()}).to_string())
        }
        "cases_salle" => {
            let (salle_id, _, _) = salle_par_nom(pool, arg(a, "salle")).await?;
            let lignes: Vec<(i64, String, String, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
                "SELECT c.id,s.nom,c.nom,c.num_vanne,COALESCE(c.nb_max_truies,c.nb_max_porcs), \
                 (SELECT group_concat(t.num_travail,', ') FROM truie t WHERE t.case_id=c.id AND t.reformee=0) \
                 FROM casesalle c JOIN salle s ON s.id=c.salle_id WHERE c.salle_id=? ORDER BY c.id",
            ).bind(salle_id).fetch_all(pool).await.map_err(|e| e.to_string())?;
            let mut sortie = Vec::new();
            for (id, s, c, v, p, t) in lignes {
                let truies: Vec<&str> = t.as_deref().map(|t| t.split(", ").collect()).unwrap_or_default();
                let porcs = case_pig_count(pool, id).await.unwrap_or(0);
                sortie.push(json!({"salle": s, "case": c, "vanne": v, "places": p, "occupees": truies.len(), "truies": truies, "porcs": porcs}));
            }
            Ok(Value::Array(sortie).to_string())
        }
        "etat_salle" => {
            let salles: Vec<(i64, String, Option<String>)> = match arg(a, "salle") {
                "" => sqlx::query_as("SELECT id,nom,type FROM salle ORDER BY ordre,nom").fetch_all(pool).await.map_err(|e| e.to_string())?,
                s => vec![salle_par_nom(pool, s).await?],
            };
            let mut sortie = Vec::new();
            for (id, nom, type_salle) in &salles {
                sortie.push(etat_salle(pool, *id, nom, type_salle.as_deref()).await);
            }
            let demon = super::ambiance::etat_demon(pool).await;
            Ok(json!({"regulation": demon["texte"], "salles": sortie}).to_string())
        }
        "que_sais_tu_faire" => Ok(FAMILLES.to_string()),
        "forcer_ventilation" => {
            let (salle_id, nom, _) = salle_par_nom(pool, arg(a, "salle")).await?;
            let pct = a.get("pourcentage").and_then(Value::as_f64).ok_or("Pourcentage manquant (0 à 100).")?;
            if !(0.0..=100.0).contains(&pct) { return Err("Le pourcentage va de 0 à 100.".into()); }
            let minutes = match a.get("duree_minutes") { None | Some(Value::Null) => 120, Some(v) => v.as_i64().ok_or("Durée en minutes attendue.")? };
            if minutes < 1 { return Err("Durée d'au moins 1 minute.".into()); }
            if minutes > crate::regul_core::FORCAGE_MAX_MIN {
                return Err(format!("Un forçage dure 24 h au plus ({minutes} min demandées). Pour un changement durable, utilise regler_ambiance."));
            }
            if !demon_vivant(pool).await {
                return Err("Le service de régulation (eo-regul) ne donne pas signe de vie : le forçage ne pourrait pas se terminer tout seul. Refusé ; à vérifier sur la page Ambiance.".into());
            }
            let ambiance = ambiance_de(pool, salle_id).await?;
            let regulee = ambiance.as_ref().is_some_and(|l| l.0 != 0);
            let fin_ts = chrono::Utc::now().timestamp() + minutes * 60;
            let fin = crate::regul_core::instant_utc(fin_ts);
            let (resume, variateur, retour) = if regulee {
                let l = ambiance.as_ref().unwrap();
                if l.1 == 0 { return Err(format!("{nom} est en observation seulement (ventilation non pilotée par EO_Sevel) : rien à forcer. Coche « Piloter la ventilation » dans Ambiance.")); }
                if l.3.as_deref().unwrap_or("").trim().is_empty() { return Err(format!("{nom} n'a pas de sortie de ventilation réglée dans Ambiance.")); }
                (format!("{nom} forcée à {pct:.0} % pendant {} (jusqu'à {}), puis la régulation reprend toute seule. Appliqué par eo-regul dans la minute.", duree_texte(minutes), heure_locale(&fin)), None, None)
            } else {
                let chemin = variateur_zigbee(pool, salle_id).await?
                    .ok_or_else(|| format!("{nom} n'est ni régulée ni équipée d'un variateur Zigbee rangé pour la ventilation : rien à forcer."))?;
                let actuel = match ambiance.as_ref().and_then(|l| l.6) { Some(r) if l_forcage_actif(ambiance.as_ref()) => Some(r), _ => pourcent_variateur(pool, &chemin).await };
                (format!("{nom} : variateur mis à {pct:.0} % pendant {} (jusqu'à {}), puis remis à {}. Envoyé au Zigbee tout de suite.",
                    duree_texte(minutes), heure_locale(&fin), actuel.map(|v| format!("{v:.0} %")).unwrap_or("sa valeur d'avant (inconnue pour l'instant)".into())), Some(chemin), actuel)
            };
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est fait) : {resume}")); }
            if let Some(chemin) = &variateur {
                sqlx::query("INSERT INTO ambiance_salle(salle_id,actif,pilote,ipx_hote,sortie_ventil) VALUES(?,0,0,'zigbee',?) ON CONFLICT(salle_id) DO UPDATE SET sortie_ventil=COALESCE(NULLIF(ambiance_salle.sortie_ventil,''),excluded.sortie_ventil), ipx_hote=COALESCE(NULLIF(ambiance_salle.ipx_hote,''),'zigbee')")
                    .bind(salle_id).bind(chemin).execute(pool).await.map_err(|e| e.to_string())?;
                commander_variateur(pool, chemin, pct).await?;
            }
            sqlx::query("UPDATE ambiance_salle SET forcage_pct=?, forcage_fin=?, forcage_retour=COALESCE(?,forcage_retour) WHERE salle_id=?")
                .bind(pct).bind(&fin).bind(retour).bind(salle_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "forcer_ventilation", &nom, &resume).await;
            Ok(format!("Fait : {resume}"))
        }
        "arreter_forcage" => {
            let (salle_id, nom, _) = salle_par_nom(pool, arg(a, "salle")).await?;
            let ambiance = ambiance_de(pool, salle_id).await?;
            if !l_forcage_actif(ambiance.as_ref()) { return Err(format!("Pas de forçage en cours en {nom}.")); }
            let l = ambiance.unwrap();
            let regulee = l.0 != 0 && l.1 != 0;
            let resume = if regulee { format!("{nom} : fin du forçage, la régulation reprend dans la minute.") }
                else { format!("{nom} : fin du forçage, variateur remis à {}.", l.6.map(|v| format!("{v:.0} %")).unwrap_or("— valeur d'avant inconnue, à régler à la main".into())) };
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est fait) : {resume}")); }
            if let (false, Some(retour), Some(chemin)) = (regulee, l.6, l.3.as_deref()) { commander_variateur(pool, chemin, retour).await?; }
            sqlx::query("UPDATE ambiance_salle SET forcage_pct=NULL, forcage_fin=NULL, forcage_retour=NULL WHERE salle_id=?")
                .bind(salle_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "arreter_forcage", &nom, &resume).await;
            Ok(format!("Fait : {resume}"))
        }
        "regler_ambiance" => {
            let (salle_id, nom, _) = salle_par_nom(pool, arg(a, "salle")).await?;
            let actuel: Option<(Option<i64>, f64, f64, f64, f64)> = sqlx::query_as("SELECT courbe_id,t_cible_fixe,plage,ventil_min,ventil_max FROM ambiance_salle WHERE salle_id=?")
                .bind(salle_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (courbe, t0, p0, vmin0, vmax0) = actuel.ok_or_else(|| format!("{nom} n'a pas de réglage d'ambiance : à créer dans la page Ambiance d'abord."))?;
            let nombre = |k: &str| a.get(k).and_then(Value::as_f64);
            let (t, p, vmin, vmax) = (nombre("t_cible").unwrap_or(t0), nombre("plage").unwrap_or(p0), nombre("ventil_min").unwrap_or(vmin0), nombre("ventil_max").unwrap_or(vmax0));
            if nombre("t_cible").is_none() && nombre("plage").is_none() && nombre("ventil_min").is_none() && nombre("ventil_max").is_none() {
                return Err(format!("Rien à changer. Actuellement en {nom} : consigne {t0} °C{}, plage {p0} °C, ventilation {vmin0} à {vmax0} %.", if courbe.is_some() { " (courbe)" } else { "" }));
            }
            if !(5.0..=40.0).contains(&t) { return Err("Consigne entre 5 et 40 °C.".into()); }
            if !(0.1..=20.0).contains(&p) { return Err("Plage entre 0,1 et 20 °C.".into()); }
            if !(0.0..=100.0).contains(&vmin) || !(0.0..=100.0).contains(&vmax) || vmin > vmax { return Err("Ventilation entre 0 et 100 %, minimum ≤ maximum.".into()); }
            let mut changements = Vec::new();
            if t != t0 { changements.push(format!("consigne {t0} → {t} °C{}", if courbe.is_some() { " (sans effet tant que la salle suit une courbe)" } else { "" })); }
            if p != p0 { changements.push(format!("plage {p0} → {p} °C")); }
            if vmin != vmin0 { changements.push(format!("ventilation mini {vmin0} → {vmin} %{}", if courbe.is_some() { " (la courbe peut l'imposer)" } else { "" })); }
            if vmax != vmax0 { changements.push(format!("ventilation maxi {vmax0} → {vmax} %")); }
            if changements.is_empty() { return Ok(format!("Déjà réglé ainsi en {nom}.")); }
            let resume = format!("{nom} : {}.", changements.join(", "));
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}")); }
            sqlx::query("UPDATE ambiance_salle SET t_cible_fixe=?,plage=?,ventil_min=?,ventil_max=? WHERE salle_id=?")
                .bind(t).bind(p).bind(vmin).bind(vmax).bind(salle_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "regler_ambiance", &nom, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "calendrier_ia" => {
            let c = load_band_schedule(pool).await.map_err(|e| e.to_string())?;
            let bandes: Vec<(String, String)> = sqlx::query_as("SELECT code,date_mb FROM bande WHERE active=1 AND date_mb IS NOT NULL ORDER BY date_mb")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            let auj = chrono::Local::now().date_naive();
            let mut sortie = Vec::new();
            for (code, mb) in bandes {
                let Ok(mb) = chrono::NaiveDate::parse_from_str(&mb, "%Y-%m-%d") else { continue };
                let ia = mb - chrono::Duration::days(c.gestation);
                let sevrage = mb + chrono::Duration::days(c.weaning);
                let ia_sevrees = sevrage + chrono::Duration::days(7 - sevrage.weekday().num_days_from_monday() as i64);
                sortie.push(json!({"bande": code, "mise_bas": mb.to_string(), "ia_de_la_bande": ia.to_string(),
                                   "sevrage": sevrage.to_string(), "ia_des_sevrees": ia_sevrees.to_string(),
                                   "ia_des_sevrees_a_venir": ia_sevrees >= auj}));
            }
            Ok(json!({"aujourdhui": auj.to_string(), "gestation_jours": c.gestation, "sevrage_jours": c.weaning, "bandes": sortie}).to_string())
        }
        "sevrer_salle" => {
            let date = date_ou_aujourdhui(a)?;
            let salles: Vec<(i64, String)> = sqlx::query_as(
                "SELECT id,nom FROM salle WHERE lower(nom) LIKE '%'||lower(?1)||'%' AND lower(COALESCE(type,'')||' '||nom) LIKE '%mater%'",
            ).bind(arg(a, "salle")).fetch_all(pool).await.map_err(|e| e.to_string())?;
            let (salle_id, salle) = match salles.as_slice() {
                [une] => une.clone(),
                [] => return Err(format!("Pas de maternité « {} ».", arg(a, "salle"))),
                _ => return Err(format!("Plusieurs maternités correspondent : {}. Laquelle ?", salles.iter().map(|s| s.1.as_str()).collect::<Vec<_>>().join(", "))),
            };
            // Cases de post-sevrage, dans l'ordre dit : par leur nom (« PS1 », « PS 2 »)
            // ou par leur salle quand elle n'a qu'une case.
            let mut cases: Vec<(i64, String, i64, Option<i64>)> = Vec::new();
            for nom in arg(a, "destination").replace(" et ", ",").replace('+', ",").split(',').map(str::trim).filter(|n| !n.is_empty()) {
                let d = nom.to_lowercase().replace(' ', "");
                let trouvees: Vec<(i64, String, Option<i64>)> = sqlx::query_as(
                    "SELECT c.id,c.nom,c.nb_max_porcs FROM casesalle c JOIN salle s ON s.id=c.salle_id \
                     WHERE (lower(COALESCE(s.type,'')) LIKE '%sevr%' OR lower(s.nom) LIKE '%sevr%') \
                     AND (replace(lower(c.nom),' ','')=?1 OR (replace(lower(s.nom),' ','') LIKE '%'||?1||'%' AND (SELECT COUNT(*) FROM casesalle x WHERE x.salle_id=s.id)=1))",
                ).bind(&d).fetch_all(pool).await.map_err(|e| e.to_string())?;
                let (id, case, max) = match trouvees.as_slice() {
                    [une] => une.clone(),
                    [] => return Err(format!("Pas de case de post-sevrage « {nom} ».")),
                    _ => return Err(format!("Plusieurs cases correspondent à « {nom} » : {}. Laquelle ?", trouvees.iter().map(|c| c.1.as_str()).collect::<Vec<_>>().join(", "))),
                };
                let deja = case_pig_count(pool, id).await.map_err(|e| e.to_string())?.max(0);
                cases.push((id, case, deja, max.filter(|m| *m > 0)));
            }
            if cases.is_empty() {
                return Err("Dans quelle case de post-sevrage vont les porcelets ?".into());
            }
            let st = match arg(a, "salle_truies") { "" => "verrat", s => s };
            let (salle_truies_id, salle_truies): (i64, String) = sqlx::query_as(
                "SELECT id,nom FROM salle WHERE lower(COALESCE(type,'')||' '||nom) LIKE '%'||lower(?1)||'%' ORDER BY ordre LIMIT 1",
            ).bind(st).fetch_optional(pool).await.map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Pas de salle « {st} » pour les truies."))?;
            // Portées ouvertes de la maternité, par bande.
            let portees: Vec<(i64, i64, String, i64)> = sqlx::query_as(
                "SELECT pe.bande_id,t.id,t.num_travail,pe.presents FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id \
                 JOIN evenement e ON e.id=pe.id JOIN casesalle c ON c.id=COALESCE(e.case_id,t.case_id) \
                 WHERE c.salle_id=? AND pe.cloturee=0 AND pe.date<=? ORDER BY pe.bande_id,CAST(t.num_travail AS INTEGER)",
            ).bind(salle_id).bind(&date).fetch_all(pool).await.map_err(|e| e.to_string())?;
            if portees.is_empty() {
                return Err(format!("Aucune portée à sevrer en {salle}."));
            }
            for p in &portees {
                let reformee: i64 = sqlx::query_scalar("SELECT reformee FROM truie WHERE id=?").bind(p.1).fetch_one(pool).await.unwrap_or(0);
                if reformee != 0 {
                    return Err(format!("Truie {} réformée mais sa portée est encore ouverte ({} présents) : lance diagnostiquer_portees sur {salle}, puis cloturer_portee après accord.", p.2, p.3));
                }
                if p.3 < 0 {
                    return Err(format!("Truie {} : portée à {} présents, chiffres incohérents : lance diagnostiquer_portees sur {salle} et corrige avant de sevrer.", p.2, p.3));
                }
            }
            // Chaque portée entière va dans la première case qui a encore la place.
            let mut remplissage: Vec<i64> = cases.iter().map(|c| c.2).collect();
            let mut affectation = Vec::new();
            for p in &portees {
                let i = (0..cases.len()).find(|&i| cases[i].3.is_none_or(|m| remplissage[i] + p.3 <= m))
                    .ok_or_else(|| format!("Plus de place pour la portée de {} ({} porcelets) : {}. Ajoute une case de post-sevrage.",
                        p.2, p.3, cases.iter().zip(&remplissage).map(|(c, r)| format!("{} {r}/{}", c.1, c.3.map(|m| m.to_string()).unwrap_or("?".into()))).collect::<Vec<_>>().join(", ")))?;
                remplissage[i] += p.3;
                affectation.push(i);
            }
            let total: i64 = portees.iter().map(|p| p.3).sum();
            let repartition = cases.iter().enumerate().filter(|(i, _)| affectation.contains(i)).map(|(i, c)| format!("{} : {} porcelet(s), {}{} après",
                c.1, remplissage[i] - c.2, remplissage[i], c.3.map(|m| format!("/{m}")).unwrap_or_default())).collect::<Vec<_>>().join(" ; ");
            let resume = format!(
                "Sevrage de {salle} le {date} : {} truie(s) ({}), {total} porcelet(s) → {repartition} ; truies → {salle_truies}.",
                portees.len(), portees.iter().map(|p| format!("{} : {}", p.2, p.3)).collect::<Vec<_>>().join(", "));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let mut bandes: Vec<i64> = portees.iter().map(|p| p.0).collect();
            bandes.dedup();
            for bande in bandes {
                let mut form: HashMap<String, String> = HashMap::from([
                    ("date".into(), date.clone()),
                    ("salle_truies".into(), salle_truies_id.to_string()),
                ]);
                for (p, i) in portees.iter().zip(&affectation).filter(|(p, _)| p.0 == bande) {
                    form.insert(format!("sel_{}", p.1), "on".into());
                    form.insert(format!("nb_{}", p.1), p.3.to_string());
                    form.insert(format!("case_{}", p.1), cases[*i].0.to_string());
                }
                super::sevrage::sevrer(pool, AUTEUR, bande, &form).await.map_err(|e| e.to_string())?;
            }
            journaliser(pool, "sevrer_salle", &salle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "diagnostiquer_portees" => {
            let (filtre, nom) = match (arg(a, "truie"), arg(a, "salle")) {
                (t, _) if !t.is_empty() => { let (id, num) = truie_par_numero_toute(pool, t).await?; (format!("t.id={id}"), format!("truie {num}")) }
                (_, s) if !s.is_empty() => { let (id, nom, _) = salle_par_nom(pool, s).await?; (format!("c.salle_id={id}"), nom) }
                _ => return Err("Indique une salle de maternité ou une truie.".into()),
            };
            let portees: Vec<(i64, i64, String, i64, String, i64, i64, i64, i64, i64, i64, i64, i64, Option<i64>, Option<i64>)> = sqlx::query_as(&format!(
                "SELECT pe.id,t.id,t.num_travail,t.reformee,pe.date,COALESCE(e.nes_totaux,0),pe.nes_vifs_calc,COALESCE(e.mort_nes,0),pe.adoptes,pe.retires,pe.pertes,pe.presents,pe.cloturee,pe.sevrage_id,pe.nb_sevres \
                 FROM (SELECT p.*,COALESCE((SELECT nes_vifs FROM evenement WHERE id=p.id),0) AS nes_vifs_calc FROM portee_effectif p) pe \
                 JOIN truie t ON t.id=pe.truie_id JOIN evenement e ON e.id=pe.id LEFT JOIN casesalle c ON c.id=COALESCE(e.case_id,t.case_id) \
                 WHERE {filtre} AND pe.date<=date('now','localtime') AND (pe.cloturee=0 OR pe.date>=date('now','localtime','-60 day')) ORDER BY pe.cloturee,t.num_travail,pe.date DESC"
            )).fetch_all(pool).await.map_err(|e| e.to_string())?;
            if portees.is_empty() { return Ok(format!("Aucune portée récente pour {nom}.")); }
            let mut sortie = Vec::new();
            let mut total_anomalies = 0;
            for (mb_id, _truie_id, num, reformee, date_mb, nt, nv, mn, adoptes, retires, pertes, presents, cloturee, sevrage_id, nb_sevres) in portees {
                let mut anomalies = Vec::new();
                if cloturee == 0 && reformee != 0 {
                    anomalies.push(format!("truie réformée avec portée ouverte → cloturer_portee truie {num} (nb_sevres 0 si ses porcelets sont déjà comptés ailleurs)"));
                }
                if cloturee == 0 && presents < 0 {
                    anomalies.push(format!("présents négatifs ({presents}) → corriger_mise_bas truie {num} nes_vifs {} (et nes_totaux {})", nv - presents, nt - presents));
                }
                let avant: Vec<(i64, String, i64)> = sqlx::query_as("SELECT id,date,nb FROM perteporcelet WHERE evenement_id=? AND date<? ORDER BY date,id")
                    .bind(mb_id).bind(&date_mb).fetch_all(pool).await.unwrap_or_default();
                if !avant.is_empty() {
                    anomalies.push(format!("pertes datées avant la mise-bas du {date_mb} : {} → corriger_perte id {} date {date_mb} (ou corriger_mise_bas date si c'est la mise-bas qui est mal datée)",
                        avant.iter().map(|(id, d, n)| format!("n° {id} le {d} ({n})")).collect::<Vec<_>>().join(", "),
                        avant.iter().map(|(id, _, _)| id.to_string()).collect::<Vec<_>>().join(", ")));
                }
                if retires > nv + adoptes {
                    anomalies.push(format!("retirés ({retires}) au-delà des nés vivants + adoptés ({}) → vérifier les adoptions ou corriger_mise_bas nes_vifs", nv + adoptes));
                }
                if sevrage_id.is_some() && nb_sevres.is_none() {
                    anomalies.push("sevrage enregistré sans nombre de sevrés → corriger dans EO_Sevel (page de la truie)".to_string());
                }
                total_anomalies += anomalies.len();
                sortie.push(json!({"truie": num, "reformee": reformee != 0, "mise_bas": date_mb, "mise_bas_id": mb_id,
                    "nes_totaux": nt, "nes_vifs": nv, "mort_nes": mn, "adoptes": adoptes, "retires": retires, "pertes": pertes,
                    "presents": presents, "ouverte": cloturee == 0, "nb_sevres": nb_sevres, "anomalies": anomalies}));
            }
            Ok(json!({"objet": nom, "anomalies": total_anomalies, "portees": sortie,
                "conseil": if total_anomalies == 0 { "Rien d'anormal." } else { "Propose chaque correction à l'éleveur, puis applique-la avec l'outil indiqué après son accord." }}).to_string())
        }
        "cloturer_portee" => {
            let (truie_id, num) = truie_par_numero_toute(pool, arg(a, "truie")).await?;
            let date = date_ou_aujourdhui(a)?;
            let nb = a.get("nb_sevres").and_then(Value::as_i64).unwrap_or(0);
            if nb < 0 { return Err("nb_sevres ne peut pas être négatif.".into()); }
            let portee: Option<(i64, Option<i64>, String, i64)> = sqlx::query_as(
                "SELECT id,bande_id,date,presents FROM portee_effectif WHERE truie_id=? AND cloturee=0 AND date<=date('now','localtime') ORDER BY date DESC,id DESC LIMIT 1",
            ).bind(truie_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (_, bande_id, date_mb, presents) = portee.ok_or_else(|| format!("Truie {num} : aucune portée ouverte."))?;
            if date < date_mb { return Err(format!("La clôture ({date}) ne peut pas précéder la mise-bas ({date_mb}).")); }
            let raison = arg(a, "note");
            let resume = format!("Portée de {num} (mise-bas du {date_mb}, {presents} présents calculés) clôturée le {date} avec {nb} sevré(s){}.", if raison.is_empty() { String::new() } else { format!(" — {raison}") });
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}")); }
            sqlx::query("INSERT INTO evenement(type,date,truie_id,bande_id,nb_sevres,note,suivi_actif) VALUES('sevrage',?,?,?,?,?,0)")
                .bind(&date).bind(truie_id).bind(bande_id).bind(nb).bind(format!("Clôture (Claude){}", if raison.is_empty() { String::new() } else { format!(" : {raison}") }))
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "cloturer_portee", &num, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "corriger_mise_bas" => {
            let (truie_id, num) = truie_par_numero_toute(pool, arg(a, "truie")).await?;
            let raison = arg(a, "note");
            if raison.is_empty() { return Err("La note (raison de la correction) est obligatoire.".into()); }
            let mb: Option<(i64, String, Option<i64>, Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
                "SELECT id,date,nes_totaux,nes_vifs,mort_nes,note FROM evenement WHERE truie_id=? AND type='mise_bas' AND date<=date('now','localtime') ORDER BY date DESC,id DESC LIMIT 1",
            ).bind(truie_id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (mb_id, date0, nt0, nv0, mn0, note0) = mb.ok_or_else(|| format!("Truie {num} : aucune mise-bas."))?;
            let entier = |k: &str| a.get(k).and_then(Value::as_i64);
            let (nt, nv, mn) = (entier("nes_totaux").or(nt0), entier("nes_vifs").or(nv0), entier("mort_nes").or(mn0));
            let date = match arg(a, "date") { "" => date0.clone(), d => { chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?; d.to_string() } };
            if [nt, nv, mn].iter().flatten().any(|v| *v < 0) { return Err("Les chiffres ne peuvent pas être négatifs.".into()); }
            if let (Some(nt), Some(nv)) = (nt, nv) { if nv > nt { return Err(format!("Nés vivants ({nv}) supérieurs aux nés totaux ({nt}).")); } }
            let mut changements = Vec::new();
            let aff = |v: Option<i64>| v.map(|v| v.to_string()).unwrap_or("—".into());
            if nv != nv0 { changements.push(format!("nés vivants {} → {}", aff(nv0), aff(nv))); }
            if nt != nt0 { changements.push(format!("nés totaux {} → {}", aff(nt0), aff(nt))); }
            if mn != mn0 { changements.push(format!("morts-nés {} → {}", aff(mn0), aff(mn))); }
            if date != date0 { changements.push(format!("date {date0} → {date}")); }
            if changements.is_empty() { return Err(format!("Rien à changer sur la mise-bas de {num} du {date0} ({} nés totaux, {} vivants, {} morts-nés).", aff(nt0), aff(nv0), aff(mn0))); }
            let presents0: i64 = sqlx::query_scalar("SELECT presents FROM portee_effectif WHERE id=?").bind(mb_id).fetch_one(pool).await.unwrap_or(0);
            let resume = format!("Mise-bas de {num} du {date0} : {} — {raison}. Présents avant : {presents0}, après : {}.", changements.join(", "), presents0 + nv.unwrap_or(0) - nv0.unwrap_or(0));
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}")); }
            let note = format!("{}Corrigé (Claude) : {} — {raison}", note0.map(|n| format!("{n} · ")).unwrap_or_default(), changements.join(", "));
            sqlx::query("UPDATE evenement SET nes_totaux=?,nes_vifs=?,mort_nes=?,date=?,note=? WHERE id=?")
                .bind(nt).bind(nv).bind(mn).bind(&date).bind(note).bind(mb_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "corriger_mise_bas", &num, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "corriger_perte" => {
            let id = a.get("id").and_then(Value::as_i64).ok_or("Numéro de perte manquant (voir diagnostiquer_portees).")?;
            let perte: Option<(Option<i64>, i64, Option<String>, Option<String>, Option<i64>)> = sqlx::query_as("SELECT truie_id,nb,date,cause,evenement_id FROM perteporcelet WHERE id=?")
                .bind(id).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let (truie_id, nb, date0, cause, mb_id) = perte.ok_or_else(|| format!("Pas de perte n° {id}."))?;
            let num: String = match truie_id { Some(t) => sqlx::query_scalar("SELECT num_travail FROM truie WHERE id=?").bind(t).fetch_optional(pool).await.ok().flatten().unwrap_or("?".into()), None => "?".into() };
            let supprimer = a.get("supprimer").and_then(Value::as_bool) == Some(true);
            let raison = arg(a, "note");
            let date = arg(a, "date");
            if !supprimer && date.is_empty() { return Err("Indique la nouvelle date, ou supprimer=true.".into()); }
            if !date.is_empty() { chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?; }
            if let (false, Some(mb)) = (supprimer, mb_id) {
                let date_mb: Option<String> = sqlx::query_scalar("SELECT date FROM evenement WHERE id=?").bind(mb).fetch_optional(pool).await.ok().flatten();
                if date_mb.as_deref().is_some_and(|d| date < d) { return Err(format!("La perte resterait avant la mise-bas du {}.", date_mb.unwrap_or_default())); }
            }
            let resume = if supprimer { format!("Perte n° {id} de {num} ({nb} porcelet(s), {}, le {}) supprimée{}.", cause.as_deref().unwrap_or("sans cause"), date0.as_deref().unwrap_or("?"), if raison.is_empty() { String::new() } else { format!(" — {raison}") }) }
                else { format!("Perte n° {id} de {num} ({nb} porcelet(s)) : date {} → {date}{}.", date0.as_deref().unwrap_or("?"), if raison.is_empty() { String::new() } else { format!(" — {raison}") }) };
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}")); }
            if supprimer { sqlx::query("DELETE FROM perteporcelet WHERE id=?").bind(id).execute(pool).await.map_err(|e| e.to_string())?; }
            else {
                sqlx::query("UPDATE perteporcelet SET date=?, age_j=CASE WHEN ? IS NOT NULL THEN CAST(julianday(?)-julianday((SELECT date FROM evenement WHERE id=?)) AS INTEGER) ELSE age_j END WHERE id=?")
                    .bind(date).bind(mb_id).bind(date).bind(mb_id).bind(id).execute(pool).await.map_err(|e| e.to_string())?;
            }
            journaliser(pool, "corriger_perte", &num, &format!("(Claude) {resume}")).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "inventaire_case" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).filter(|n| *n >= 0).ok_or("Nombre manquant (0 ou plus).")?;
            let (salle_id, nom, _) = salle_par_nom(pool, arg(a, "salle")).await?;
            let cases: Vec<(i64, String)> = sqlx::query_as(
                "SELECT id,nom FROM casesalle WHERE salle_id=?1 AND (?2='' OR num_vanne=?2 OR lower(nom)=lower(?2) OR replace(lower(nom),' ','')=replace(lower(?2),' ','')) ORDER BY id",
            ).bind(salle_id).bind(arg(a, "case").trim_start_matches(['v', 'V']).trim()).fetch_all(pool).await.map_err(|e| e.to_string())?;
            let (case_id, case) = match cases.as_slice() {
                [une] => une.clone(),
                [] => return Err(format!("Pas de case « {} » en {nom}.", arg(a, "case"))),
                plusieurs => return Err(format!("{nom} a plusieurs cases, précise : {}.", plusieurs.iter().map(|c| c.1.as_str()).collect::<Vec<_>>().join(", "))),
            };
            let avant = case_pig_count(pool, case_id).await.map_err(|e| e.to_string())?;
            let raison = arg(a, "note");
            let resume = format!("Inventaire de {nom}, case {case}, le {date} : {nombre} porcs comptés (EO_Sevel en calculait {avant}){}.", if raison.is_empty() { String::new() } else { format!(" — {raison}") });
            if !confirme(a) { return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}")); }
            sqlx::query("INSERT INTO inventairecase(case_id,date,nombre,note,cree_par,dernier_transfert_id) VALUES(?,?,?,?,?,(SELECT MAX(id) FROM transfert))")
                .bind(case_id).bind(&date).bind(nombre).bind(format!("(Claude){}", if raison.is_empty() { String::new() } else { format!(" {raison}") })).bind(AUTEUR)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "inventaire_case", &nom, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "enregistrer_achat" => {
            let categorie = arg(a, "categorie");
            let montant = a.get("montant_ht").and_then(Value::as_f64).filter(|m| *m > 0.0)
                .ok_or("Montant HT manquant.")?;
            let date = date_ou_aujourdhui(a)?;
            let quantite = a.get("quantite").and_then(Value::as_f64);
            let texte_ou = |k: &str| Some(arg(a, k).to_string()).filter(|t| !t.is_empty());
            let (fournisseur, designation, num, note) = (texte_ou("fournisseur"), texte_ou("designation"), texte_ou("num_facture"), texte_ou("note"));
            let bande: Option<(i64, String)> = match arg(a, "bande") {
                "" => None,
                b => Some(sqlx::query_as("SELECT id,code FROM bande WHERE lower(replace(code,' ',''))=lower(replace(?,' ',''))")
                    .bind(b).fetch_optional(pool).await.map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Pas de bande « {b} ».") )?),
            };
            let libelle = match categorie {
                "semence" => "semence", "veto" => "vétérinaire", "genetique" => "génétique", "aliment" => "aliment",
                _ => return Err("Catégorie : semence, veto, genetique ou aliment.".into()),
            };
            let resume = format!("Achat {libelle} du {date} : {montant:.2} € HT{}{}{}{}.",
                fournisseur.as_deref().map(|f| format!(", {f}")).unwrap_or_default(),
                designation.as_deref().map(|d| format!(", {d}")).unwrap_or_default(),
                quantite.map(|q| format!(", quantité {q}")).unwrap_or_default(),
                bande.as_ref().map(|b| format!(", bande {}", b.1)).unwrap_or_else(|| ", sans bande".into()));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let bande_id = bande.as_ref().map(|b| b.0);
            let requete = match categorie {
                "semence" => sqlx::query("INSERT INTO achatsemence(date,num_facture,fournisseur,designation,nb_doses,montant_ht,bande_id,note) VALUES(?,?,?,?,?,?,?,?)")
                    .bind(&date).bind(&num).bind(&fournisseur).bind(&designation).bind(quantite.map(|q| q.round() as i64)).bind(montant).bind(bande_id).bind(&note),
                "veto" => sqlx::query("INSERT INTO achatveto(date,produit,quantite,montant_ht,num_facture,fournisseur,bande_id) VALUES(?,?,?,?,?,?,?)")
                    .bind(&date).bind(&designation).bind(quantite).bind(montant).bind(&num).bind(&fournisseur).bind(bande_id),
                "genetique" => sqlx::query("INSERT INTO achatgenetique(date,num_facture,fournisseur,designation,nb_animaux,montant_ht,bande_code,note) VALUES(?,?,?,?,?,?,?,?)")
                    .bind(&date).bind(&num).bind(&fournisseur).bind(&designation).bind(quantite.map(|q| q.round() as i64)).bind(montant).bind(bande.as_ref().map(|b| b.1.clone())).bind(&note),
                _ => sqlx::query("INSERT INTO livraisonaliment(date,fournisseur,produit,tonnage,montant_ht,num_facture,bande_id) VALUES(?,?,?,?,?,?,?)")
                    .bind(&date).bind(&fournisseur).bind(&designation).bind(quantite).bind(montant).bind(&num).bind(bande_id),
            };
            requete.execute(pool).await.map_err(|e| e.to_string())?;
            crate::db::auto_assign_economic_invoices(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "enregistrer_achat", libelle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "deplacer_truie" => {
            let (truie_id, num) = truie_par_numero(pool, arg(a, "numero")).await?;
            let (case_id, salle_id, salle, case, places) = case_par_vanne(pool, arg(a, "salle"), arg(a, "vanne")).await?;
            let deja: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie WHERE case_id=? AND reformee=0 AND id<>?")
                .bind(case_id).bind(truie_id).fetch_one(pool).await.map_err(|e| e.to_string())?;
            let occupation = match places {
                Some(p) if deja >= p => format!(" ATTENTION : case déjà pleine ({deja}/{p})."),
                Some(p) => format!(" Après : {}/{p} places.", deja + 1),
                None => String::new(),
            };
            let resume = format!("Truie {num} → {salle}, case {case}.{occupation}");
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("UPDATE truie SET case_id=?,salle_id=?,updated_at=CURRENT_TIMESTAMP WHERE id=?")
                .bind(case_id).bind(salle_id).bind(truie_id).execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "deplacer_truie", &num, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "creer_rappel" => {
            let titre = arg(a, "titre");
            let date = chrono::NaiveDate::parse_from_str(arg(a, "date"), "%Y-%m-%d").map_err(|_| "Date attendue au format AAAA-MM-JJ".to_string())?;
            if titre.is_empty() {
                return Err("Titre du rappel manquant.".into());
            }
            let bande = Some(arg(a, "bande")).filter(|b| !b.is_empty());
            let note = Some(arg(a, "note")).filter(|n| !n.is_empty());
            let resume = format!("Rappel « {titre} » le {}", date.format("%d/%m/%Y"));
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("INSERT INTO tache(titre,type,bande_code,echeance,fait,note,cree_le) VALUES(?,'Rappel',?,?,0,?,CURRENT_TIMESTAMP)")
                .bind(titre).bind(bande).bind(date.to_string()).bind(note)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "creer_rappel", titre, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "declarer_mort" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let cases = cases_porcs(pool, arg(a, "salle"), arg(a, "case")).await?;
            if cases.len() > 1 && arg(a, "case").is_empty() && nombre > 0 && cases[0].4 < nombre {
                return Err(format!("Précise la case : {}.", cases.iter().map(|c| format!("{} ({} porcs)", c.3, c.4)).collect::<Vec<_>>().join(", ")));
            }
            let (case_id, _, salle, case, present) = cases[0].clone();
            prelever(&cases[..1], nombre)?;
            let causes: Vec<String> = sqlx::query_scalar("SELECT libelle FROM causeperte ORDER BY libelle")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            let cause = causes.iter().find(|c| c.to_lowercase() == arg(a, "cause").to_lowercase())
                .ok_or_else(|| format!("Cause inconnue « {} ». Causes possibles : {}.", arg(a, "cause"), causes.join(", ")))?.clone();
            let stade = Some(arg(a, "stade").to_string()).filter(|s| !s.is_empty())
                .or(stade_from_case(pool, case_id).await.map_err(|e| e.to_string())?)
                .map(|s| normalize_death_stage(&s));
            let bande = code_bande(pool, bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?).await;
            let resume = format!("{nombre} porc(s) mort(s) le {date} en {salle}, case {case} ({}), cause {cause}{}. Reste {} porcs dans la case.",
                stade.as_deref().unwrap_or("stade inconnu"), bande.as_deref().map(|b| format!(", bande {b}")).unwrap_or_default(), present - nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            sqlx::query("INSERT INTO declarationmort(horodatage,bande_code,date,stade,case_id,cause,nombre,declare_par) VALUES(CURRENT_TIMESTAMP,?,?,?,?,?,?,?)")
                .bind(&bande).bind(&date).bind(&stade).bind(case_id).bind(&cause).bind(nombre).bind(AUTEUR)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "declarer_mort", &salle, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "transferer_porcs" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let sources = cases_porcs(pool, arg(a, "salle_source"), arg(a, "case_source")).await?;
            let dest = cases_porcs(pool, arg(a, "salle_destination"), arg(a, "case_destination")).await?;
            if dest.len() > 1 {
                return Err(format!("{} a plusieurs cases, précise case_destination : {}.", dest[0].2, dest.iter().map(|c| format!("{} ({} porcs)", c.3, c.4)).collect::<Vec<_>>().join(", ")));
            }
            let (dest_case, dest_salle, dest_nom, dest_case_nom, dest_n) = dest[0].clone();
            let sources: Vec<_> = sources.into_iter().filter(|c| c.0 != dest_case).collect();
            if sources.is_empty() {
                return Err("La source et la destination sont identiques.".into());
            }
            let pris = prelever(&sources, nombre)?;
            let mut lignes = Vec::new();
            for (case_id, salle_id, n) in pris {
                let bande_id = bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?;
                let code = code_bande(pool, bande_id).await;
                let voulue = arg(a, "bande");
                if !voulue.is_empty() && code.as_deref().map(|c| c.to_lowercase()) != Some(voulue.to_lowercase()) {
                    return Err(format!("Les porcs de {} sont de la bande {}, pas {voulue}.", sources[0].2, code.as_deref().unwrap_or("inconnue")));
                }
                lignes.push((case_id, salle_id, n, bande_id, code));
            }
            let bandes: std::collections::BTreeSet<String> = lignes.iter().filter_map(|l| l.4.clone()).collect();
            let resume = format!("{nombre} porc(s) de {} vers {dest_nom}, case {dest_case_nom}, le {date}{}. Après : {} porcs à l'arrivée.",
                sources[0].2, if bandes.is_empty() { String::new() } else { format!(", bande {}", bandes.into_iter().collect::<Vec<_>>().join(", ")) }, dest_n + nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            for (case_id, salle_id, n, bande_id, _) in lignes {
                sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,salle_dest_id,case_source_id,case_dest_id,nombre,note) VALUES(?,'porc',?,?,?,?,?,?,?)")
                    .bind(&date).bind(bande_id).bind(salle_id).bind(dest_salle).bind(case_id).bind(dest_case).bind(n).bind(AUTEUR)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            journaliser(pool, "transferer_porcs", &dest_nom, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "depart_porcs" => {
            let date = date_ou_aujourdhui(a)?;
            let nombre = a.get("nombre").and_then(Value::as_i64).unwrap_or(0);
            let cases = cases_porcs(pool, arg(a, "salle"), arg(a, "case")).await?;
            let pris = prelever(&cases, nombre)?;
            let present: i64 = cases.iter().map(|c| c.4).sum();
            let resume = format!("Départ abattoir le {date} : {nombre} porc(s) de {}. Reste {} porcs.", cases[0].2, present - nombre);
            if a.get("confirme").and_then(Value::as_bool) != Some(true) {
                return Ok(format!("À CONFIRMER (rien n'est enregistré) : {resume}"));
            }
            // Même forme que la confirmation d'un départ prévu (/departs) :
            // sortie sans case d'arrivée, note « Départ du … ».
            let note = format!("Départ du {} ({AUTEUR})", chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").map(|d| d.format("%d/%m").to_string()).unwrap_or_default());
            let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
            for (case_id, salle_id, n) in pris {
                let bande_id = bande_de_la_case(pool, case_id, &date).await.map_err(|e| e.to_string())?;
                sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_source_id,case_source_id,nombre,note) VALUES(?,'porc',?,?,?,?,?)")
                    .bind(&date).bind(bande_id).bind(salle_id).bind(case_id).bind(n).bind(&note)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
            }
            tx.commit().await.map_err(|e| e.to_string())?;
            journaliser(pool, "depart_porcs", &cases[0].2, &resume).await;
            Ok(format!("Enregistré : {resume}"))
        }
        "rappels" => {
            let l: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as("SELECT titre,echeance,note FROM tache WHERE fait=0 ORDER BY echeance IS NULL,echeance LIMIT 50")
                .fetch_all(pool).await.map_err(|e| e.to_string())?;
            Ok(json!(l.into_iter().map(|(t, e, n)| json!({"titre": t, "echeance": e, "note": n})).collect::<Vec<_>>()).to_string())
        }
        "memoire" => {
            let notes = notes_memoire(pool).await?;
            Ok(if notes.is_empty() { "Rien n'a encore été retenu.".into() } else { notes_en_texte(&notes) })
        }
        "retenir" => {
            let t = arg(a, "texte");
            if t.is_empty() { return Err("Texte vide.".into()); }
            if t.chars().count() > 200 { return Err("Trop long : une phrase courte (200 caractères au plus).".into()); }
            if notes_memoire(pool).await?.len() >= 40 { return Err("Déjà 40 informations retenues : fais-en oublier une d'abord (memoire puis oublier).".into()); }
            table_memoire(pool).await?;
            sqlx::query("INSERT INTO assistant_memoire(texte) VALUES(?)").bind(t)
                .execute(pool).await.map_err(|e| e.to_string())?;
            journaliser(pool, "retenir", "mémoire", t).await;
            Ok(format!("Retenu : {t}"))
        }
        "oublier" => {
            let n = a.get("numero").and_then(Value::as_i64).ok_or("Numéro manquant.")?;
            table_memoire(pool).await?;
            let r = sqlx::query("DELETE FROM assistant_memoire WHERE id=?").bind(n)
                .execute(pool).await.map_err(|e| e.to_string())?;
            if r.rows_affected() == 0 { return Err(format!("Pas d'information n° {n}.")); }
            journaliser(pool, "oublier", "mémoire", &n.to_string()).await;
            Ok(format!("Information n° {n} oubliée."))
        }
        "lister_tables" | "decrire_tables" | "requete" => {
            let mut cnx = assistant::connexion(&state.config.db_path).await?;
            let (_, r) = assistant::executer(&mut cnx, nom, a).await;
            if r.starts_with("ERREUR") { Err(r) } else { Ok(r) }
        }
        _ => Err(format!("Outil inconnu : {nom}")),
    }
}

/// Jeton valide ? Comparaison sur toute la longueur, jeton d'au moins 24 caractères.
async fn jeton_ok(pool: &SqlitePool, jeton: &str) -> bool {
    match crate::proxmox::parametre(pool, "mcp_jeton").await {
        Some(attendu) if attendu.len() >= 24 && attendu.len() == jeton.len() => {
            attendu.bytes().zip(jeton.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
        }
        _ => false,
    }
}

pub(super) async fn point_entree(
    State(state): State<AppState>,
    Path(jeton): Path<String>,
    Json(req): Json<Value>,
) -> Response {
    if !jeton_ok(&state.pool, &jeton).await {
        return StatusCode::NOT_FOUND.into_response();
    }
    let id = req.get("id").cloned();
    let methode = req["method"].as_str().unwrap_or("");
    let Some(id) = id else {
        // Notification (initialized, cancelled…) : rien à répondre.
        return StatusCode::ACCEPTED.into_response();
    };
    let resultat = match methode {
        "initialize" => {
            let demande = req["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = PROTOCOLES.iter().find(|v| **v == demande).copied().unwrap_or(PROTOCOLES[0]);
            let notes = notes_memoire(&state.pool).await.unwrap_or_default();
            let consignes = if notes.is_empty() { CONSIGNES.to_string() }
                else { format!("{CONSIGNES}\n\nDéjà retenu :\n{}", notes_en_texte(&notes)) };
            Ok(json!({"protocolVersion": version, "capabilities": {"tools": {}},
                      "serverInfo": {"name": "eo-sevel", "version": env!("CARGO_PKG_VERSION")},
                      "instructions": consignes}))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": outils()})),
        "tools/call" => {
            let nom = req["params"]["name"].as_str().unwrap_or("");
            let args = req["params"].get("arguments").cloned().unwrap_or(json!({}));
            let r = appeler(&state, nom, &args).await;
            tracing::info!(outil = nom, ok = r.is_ok(), "connecteur Claude");
            Ok(match r { Ok(t) => texte(t, false), Err(e) => texte(e, true) })
        }
        autre => Err(json!({"code": -32601, "message": format!("Méthode inconnue : {autre}")})),
    };
    let corps = match resultat {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    };
    Json(corps).into_response()
}

pub(super) async fn get_refuse() -> StatusCode {
    StatusCode::METHOD_NOT_ALLOWED
}

/// `EO_TEST_BASE=/var/tmp/copie.db cargo test connecteur_sur_copie -- --ignored --nocapture`
/// (la copie est modifiée : truie 409353 déplacée, un rappel ajouté).
#[cfg(test)]
mod tests {
    use super::*;

    async fn rpc(state: &AppState, jeton: &str, req: Value) -> (StatusCode, Value) {
        let r = point_entree(State(state.clone()), Path(jeton.to_string()), Json(req)).await;
        let statut = r.status();
        let octets = axum::body::to_bytes(r.into_body(), 1 << 20).await.unwrap();
        (statut, serde_json::from_slice(&octets).unwrap_or(Value::Null))
    }

    async fn outil(state: &AppState, jeton: &str, nom: &str, args: Value) -> (bool, String) {
        let (_, v) = rpc(state, jeton, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":nom,"arguments":args}})).await;
        let t = v["result"]["content"][0]["text"].as_str().unwrap_or("").to_string();
        println!("--- {nom} {args}\n{t}\n");
        (v["result"]["isError"] == true, t)
    }

    /// Base en mémoire avec un petit élevage : Engr2 (régulée, 80 porcs),
    /// Engr3 (variateur Zigbee réglé à la main, vide), PS1 (une case),
    /// 35G6K (deux cases), Mater 1.
    async fn etat_test() -> anyhow::Result<(AppState, SqlitePool, String)> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await?;
        for m in [include_str!("../../migrations/0001_schema.sql"), include_str!("../../migrations/0005_ambiance.sql"),
                  include_str!("../../migrations/0007_ipx_points.sql"), include_str!("../../migrations/0017_zigbee.sql"),
                  include_str!("../../migrations/0034_forcage_ventilation.sql")] {
            sqlx::raw_sql(m).execute(&pool).await?;
        }
        sqlx::query("INSERT INTO site(id,code,nom) VALUES(1,'S1','Site test')").execute(&pool).await?;
        for (id, nom, type_salle, ordre) in [(14, "Engraissement 2", "Engraissement", 4), (15, "Engraissement 3", "Engraissement", 5),
                                             (10, "Post-sevrage 1", "post_sevrage", 1), (4, "35G6K", "engraissement", 0), (7, "Mater 1", "maternite", 0)] {
            sqlx::query("INSERT INTO salle(id,site_id,nom,type,nb_cases,ordre) VALUES(?,1,?,?,1,?)").bind(id).bind(nom).bind(type_salle).bind(ordre).execute(&pool).await?;
        }
        for (id, salle, nom, vanne, max) in [(137, 14, "Engraissement 2", "4-5-6", 75), (138, 15, "Engraissement 3", "7-8-9", 75), (90, 10, "PS1", "", 120),
                                             (47, 4, "Case 1", "1", 280), (48, 4, "Case 2", "2", 204), (60, 7, "Case 1", "1", 1)] {
            sqlx::query("INSERT INTO casesalle(id,salle_id,nom,num_vanne,nb_max_porcs) VALUES(?,?,?,?,?)").bind(id).bind(salle).bind(nom).bind(vanne).bind(max).execute(&pool).await?;
        }
        sqlx::query("INSERT INTO bande(id,code,active) VALUES(3,'B3.12',1)").execute(&pool).await?;
        sqlx::query("INSERT INTO transfert(date,espece,bande_id,salle_dest_id,case_dest_id,nombre) VALUES('2026-11-01','porc',3,14,137,80)").execute(&pool).await?;
        sqlx::query("INSERT INTO causeperte(libelle) VALUES('Respiratoire')").execute(&pool).await?;
        // Engr2 régulée par Zigbee ; Engr3 : variateur rangé mais pas de régulation.
        sqlx::query("INSERT INTO ambiance_salle(salle_id,actif,pilote,ipx_hote,sortie_ventil,courbe_id,t_cible_fixe,plage,ventil_min,ventil_max) VALUES(14,1,1,'zigbee','0xa4c138a52b72cf69/brightness',NULL,20,3,20,100)").execute(&pool).await?;
        sqlx::query("INSERT INTO ambiance_etat(salle_id,ts,t,h,consigne,ventil,bande_id,jour_bande,mode,sonde_ok) VALUES(14,strftime('%s','now'),22.4,66,21.9,32,3,10,'auto',1)").execute(&pool).await?;
        sqlx::query("INSERT INTO ipx_point(hote,chemin,nom,role,salle_id) VALUES('zigbee','0xa4c138785d1ad16e/brightness','ventil-eng3 — réglage','ventilation',15)").execute(&pool).await?;
        sqlx::query("INSERT INTO zigbee_valeur(ieee,cle,valeur,ts) VALUES('0xa4c138785d1ad16e','brightness',76,strftime('%s','now')),('0xa4c138785d1ad16e','state',1,strftime('%s','now'))").execute(&pool).await?;
        sqlx::query("INSERT INTO ambiance_heartbeat(id,ts,version,cycles,erreurs) VALUES(1,strftime('%s','now'),'test',1,0)").execute(&pool).await?;
        let jeton = "a".repeat(64);
        crate::sms_free::ecrire_parametre(&pool, "mcp_jeton", &jeton).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: "data/test.db".into(), secure_cookies: false },
            pool.clone(),
            crate::templates::build()?,
        );
        Ok((state, pool, jeton))
    }

    #[test]
    fn les_noms_courts_de_salle_se_ramenent_a_une_cle() {
        for (a, b) in [("Engraissement 2", "engr2"), ("Engraissement 2", "eng 2"), ("Engraissement 2", "E2"), ("Post-sevrage 1", "PS1"),
                       ("Post-sevrage 1", "ps 1"), ("Mater 1", "maternité 1"), ("Mater 1", "mat1"), ("Verraterie", "verrat"), ("35G6K", "35g6k")] {
            assert_eq!(cle_salle(a), cle_salle(b), "{a} / {b}");
        }
        assert_ne!(cle_salle("Engraissement 2"), cle_salle("Engraissement 3"));
        assert_ne!(cle_salle("Post-Engraissement"), cle_salle("Engraissement"));
    }

    #[tokio::test]
    async fn une_salle_se_trouve_par_son_nom_court() -> anyhow::Result<()> {
        let (_, pool, _) = etat_test().await?;
        for (dit, attendu) in [("engr2", "Engraissement 2"), ("eng 3", "Engraissement 3"), ("PS1", "Post-sevrage 1"), ("post sevrage 1", "Post-sevrage 1"),
                               ("mater 1", "Mater 1"), ("35g6k", "35G6K"), ("Engraissement 2", "Engraissement 2"), ("E3", "Engraissement 3")] {
            assert_eq!(salle_par_nom(&pool, dit).await.map(|s| s.1), Ok(attendu.to_string()), "{dit}");
        }
        let e = salle_par_nom(&pool, "Engraissement").await.unwrap_err();
        assert!(e.contains("Plusieurs salles") && e.contains("Engraissement 3"), "{e}");
        let e = salle_par_nom(&pool, "Gestante").await.unwrap_err();
        assert!(e.contains("Pas de salle") && e.contains("Mater 1"), "{e}");
        Ok(())
    }

    #[tokio::test]
    async fn un_transfert_salle_a_salle_prend_la_case_unique() -> anyhow::Result<()> {
        let (state, pool, jeton) = etat_test().await?;
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"engr2","salle_destination":"eng 3","nombre":20})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("Engraissement 3") && t.contains("B3.12"), "{t}");
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"engr2","salle_destination":"eng 3","nombre":200,"confirme":true})).await;
        assert!(e && t.contains("80 porc"), "{t}");
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"engr2","salle_destination":"35G6K","nombre":20,"confirme":true})).await;
        assert!(e && t.contains("case_destination") && t.contains("Case 2"), "{t}");
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"engr2","salle_destination":"mater 1","nombre":20,"confirme":true})).await;
        assert!(e && t.contains("maternité"), "{t}");
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"engr2","salle_destination":"eng 3","nombre":20,"confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(case_pig_count(&pool, 138).await?, 20);
        assert_eq!(case_pig_count(&pool, 137).await?, 60);
        let (e, t) = outil(&state, &jeton, "cases_salle", json!({"salle":"eng3"})).await;
        assert!(!e && t.contains("\"porcs\":20"), "{t}");
        let (e, t) = outil(&state, &jeton, "declarer_mort", json!({"salle":"Eng 3","nombre":1,"cause":"respiratoire","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(case_pig_count(&pool, 138).await?, 19);
        Ok(())
    }

    #[tokio::test]
    async fn un_forcage_se_pose_puis_se_leve() -> anyhow::Result<()> {
        let (state, pool, jeton) = etat_test().await?;
        let forcage = |p: &SqlitePool| { let p = p.clone(); async move {
            sqlx::query_as::<_, (Option<f64>, Option<String>, Option<f64>)>("SELECT forcage_pct,forcage_fin,forcage_retour FROM ambiance_salle WHERE salle_id=14").fetch_one(&p).await.unwrap() } };
        // Salle régulée : deux temps, puis colonnes posées, puis levée.
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"engr2","pourcentage":100})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("2 h"), "{t}");
        assert_eq!(forcage(&pool).await.0, None);
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"engr2","pourcentage":100,"duree_minutes":1500,"confirme":true})).await;
        assert!(e && t.contains("24 h"), "{t}");
        let (e, _) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"engr2","pourcentage":130,"confirme":true})).await;
        assert!(e);
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"engr2","pourcentage":100,"duree_minutes":90,"confirme":true})).await;
        assert!(!e && t.starts_with("Fait"), "{t}");
        let (pct, fin, _) = forcage(&pool).await;
        assert_eq!(pct, Some(100.0));
        let fin = fin.unwrap();
        let attendu = crate::regul_core::instant_utc(chrono::Utc::now().timestamp() + 90 * 60);
        assert_eq!(&fin[..13], &attendu[..13], "fin {fin} attendue vers {attendu}");
        assert!(crate::regul_core::forcage_en_cours(pct, Some(&fin), &crate::regul_core::instant_utc(chrono::Utc::now().timestamp())).is_some());
        let (e, t) = outil(&state, &jeton, "etat_salle", json!({"salle":"engr2"})).await;
        assert!(!e && t.contains("forcé à 100") && t.contains("\"porcs\":80") && t.contains("B3.12"), "{t}");
        let (e, t) = outil(&state, &jeton, "arreter_forcage", json!({"salle":"engr2","confirme":true})).await;
        assert!(!e && t.starts_with("Fait"), "{t}");
        assert_eq!(forcage(&pool).await, (None, None, None));
        let (e, _) = outil(&state, &jeton, "arreter_forcage", json!({"salle":"engr2","confirme":true})).await;
        assert!(e, "plus rien à arrêter");

        // Salle non régulée avec variateur : commande Zigbee tout de suite, retour mémorisé.
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"eng 3","pourcentage":100,"duree_minutes":30,"confirme":true})).await;
        assert!(!e && t.contains("remis à 30 %"), "{t}");
        let charge: String = sqlx::query_scalar("SELECT charge FROM zigbee_commande ORDER BY id DESC LIMIT 1").fetch_one(&pool).await?;
        assert_eq!(charge, "{\"brightness\":254}");
        let ligne: (i64, Option<f64>, Option<f64>) = sqlx::query_as("SELECT actif,forcage_pct,forcage_retour FROM ambiance_salle WHERE salle_id=15").fetch_one(&pool).await?;
        assert_eq!(ligne, (0, Some(100.0), Some(30.0)));
        let (e, t) = outil(&state, &jeton, "arreter_forcage", json!({"salle":"eng 3","confirme":true})).await;
        assert!(!e, "{t}");
        let charge: String = sqlx::query_scalar("SELECT charge FROM zigbee_commande ORDER BY id DESC LIMIT 1").fetch_one(&pool).await?;
        assert_eq!(charge, "{\"brightness\":76}");

        // Sans variateur ni régulation : refus clair. Sans démon : refus.
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"PS1","pourcentage":50,"confirme":true})).await;
        assert!(e && t.contains("rien à forcer"), "{t}");
        sqlx::query("UPDATE ambiance_heartbeat SET ts=ts-3600").execute(&pool).await?;
        let (e, t) = outil(&state, &jeton, "forcer_ventilation", json!({"salle":"engr2","pourcentage":50,"confirme":true})).await;
        assert!(e && t.contains("eo-regul"), "{t}");

        // Réglage durable.
        let (e, t) = outil(&state, &jeton, "regler_ambiance", json!({"salle":"engr2","ventil_min":25,"confirme":true})).await;
        assert!(!e && t.contains("20 → 25"), "{t}");
        let vmin: f64 = sqlx::query_scalar("SELECT ventil_min FROM ambiance_salle WHERE salle_id=14").fetch_one(&pool).await?;
        assert_eq!(vmin, 25.0);
        let (e, t) = outil(&state, &jeton, "que_sais_tu_faire", json!({})).await;
        assert!(!e && t.contains("forcer_ventilation"));
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        let noms: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().filter_map(|t| t["name"].as_str()).collect();
        for n in ["forcer_ventilation", "arreter_forcage", "etat_salle", "regler_ambiance", "que_sais_tu_faire"] { assert!(noms.contains(&n), "{n}"); }
        Ok(())
    }

    /// Le cas du 11/11 : en Mater 2, la truie 500371 est réformée avec une
    /// portée ouverte et des pertes saisies avant la mise-bas (portée à −2) ;
    /// le sevrage est refusé, le diagnostic explique, les corrections passent,
    /// puis le sevrage, puis l'inventaire de PS2.
    #[tokio::test]
    async fn un_sevrage_refuse_se_diagnostique_et_se_corrige() -> anyhow::Result<()> {
        let (state, pool, jeton) = etat_test().await?;
        sqlx::query("INSERT INTO salle(id,site_id,nom,type,nb_cases,ordre) VALUES(8,1,'Mater 2','maternite',1,0),(12,1,'Post-sevrage 2','Post-sevrage',1,2),(1,1,'Verraterie','verraterie',1,-1)").execute(&pool).await?;
        sqlx::query("INSERT INTO casesalle(id,salle_id,nom,nb_max_porcs) VALUES(61,8,'Case 1',NULL),(135,12,'PS 2',120),(1,1,'Case 1',NULL)").execute(&pool).await?;
        sqlx::query("INSERT INTO bande(id,code,active,date_mb) VALUES(2,'B2.11',1,'2026-10-14')").execute(&pool).await?;
        sqlx::query("INSERT INTO truie(id,num_travail,reformee,case_id,salle_id,bande_code) VALUES(61,'500371',1,61,8,'B2.11'),(82,'500696',0,61,8,'B2.11')").execute(&pool).await?;
        sqlx::query("INSERT INTO evenement(id,type,date,truie_id,bande_id,nes_totaux,nes_vifs,mort_nes,case_id,suivi_actif) VALUES(108,'mise_bas','2026-10-14',61,2,1,1,0,61,0),(109,'mise_bas','2026-10-14',82,2,13,11,1,61,0)").execute(&pool).await?;
        sqlx::query("INSERT INTO perteporcelet(id,truie_id,bande_id,nb,cause,date,evenement_id) VALUES(60,61,2,2,'Écrasé','2026-10-12',108),(61,61,2,1,'Chétif','2026-10-13',108),(63,82,2,1,'Chétif','2026-10-15',109)").execute(&pool).await?;
        let presents = |p: &SqlitePool, id: i64| { let p = p.clone(); async move { sqlx::query_scalar::<_, i64>("SELECT presents FROM portee_effectif WHERE id=?").bind(id).fetch_one(&p).await.unwrap() } };
        assert_eq!(presents(&pool, 108).await, -2);
        assert_eq!(presents(&pool, 109).await, 10);

        // Refus explicite.
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 2","destination":"PS 2"})).await;
        assert!(e && t.contains("500371") && t.contains("diagnostiquer_portees"), "{t}");

        // Diagnostic : les trois anomalies et leurs corrections.
        let (e, t) = outil(&state, &jeton, "diagnostiquer_portees", json!({"salle":"mater 2"})).await;
        assert!(!e, "{t}");
        let v: Value = serde_json::from_str(&t)?;
        assert_eq!(v["anomalies"], 2 + 0 + 1, "{t}");
        let p371 = v["portees"].as_array().unwrap().iter().find(|p| p["truie"] == "500371").unwrap().clone();
        let anomalies = p371["anomalies"].to_string();
        assert!(anomalies.contains("réformée") && anomalies.contains("cloturer_portee"), "{anomalies}");
        assert!(anomalies.contains("négatifs (-2)") && anomalies.contains("nes_vifs 3"), "{anomalies}");
        assert!(anomalies.contains("avant la mise-bas") && anomalies.contains("n° 60") && anomalies.contains("corriger_perte id 60, 61"), "{anomalies}");
        let (e, t) = outil(&state, &jeton, "diagnostiquer_portees", json!({"truie":"500696"})).await;
        assert!(!e && t.contains("\"anomalies\":0"), "{t}");

        // Corrections, chacune en deux temps.
        let (e, t) = outil(&state, &jeton, "corriger_perte", json!({"id":60,"date":"2026-10-14"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("2026-10-12 → 2026-10-14"), "{t}");
        let (e, _) = outil(&state, &jeton, "corriger_perte", json!({"id":60,"date":"2026-10-10","confirme":true})).await;
        assert!(e, "pas avant la mise-bas");
        for id in [60, 61] {
            let (e, t) = outil(&state, &jeton, "corriger_perte", json!({"id":id,"date":"2026-10-14","note":"saisie avant la mise-bas","confirme":true})).await;
            assert!(!e && t.starts_with("Enregistré"), "{t}");
        }
        let age: i64 = sqlx::query_scalar("SELECT age_j FROM perteporcelet WHERE id=60").fetch_one(&pool).await?;
        assert_eq!(age, 0);
        let (e, _) = outil(&state, &jeton, "corriger_mise_bas", json!({"truie":"500371","nes_vifs":3,"confirme":true})).await;
        assert!(e, "note obligatoire");
        let (e, t) = outil(&state, &jeton, "corriger_mise_bas", json!({"truie":"500371","nes_vifs":3,"nes_totaux":3,"note":"pertes comptées avant la mise-bas"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("nés vivants 1 → 3") && t.contains("après : 0"), "{t}");
        assert_eq!(presents(&pool, 108).await, -2);
        let (e, t) = outil(&state, &jeton, "corriger_mise_bas", json!({"truie":"500371","nes_vifs":3,"nes_totaux":3,"note":"pertes comptées avant la mise-bas","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(presents(&pool, 108).await, 0);
        let note: String = sqlx::query_scalar("SELECT note FROM evenement WHERE id=108").fetch_one(&pool).await?;
        assert!(note.contains("(Claude)") && note.contains("pertes comptées"), "{note}");
        let (e, t) = outil(&state, &jeton, "cloturer_portee", json!({"truie":"500371","note":"truie réformée"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("0 sevré"), "{t}");
        let (e, t) = outil(&state, &jeton, "cloturer_portee", json!({"truie":"500371","note":"truie réformée","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        let cloturee: i64 = sqlx::query_scalar("SELECT cloturee FROM portee_effectif WHERE id=108").fetch_one(&pool).await?;
        assert_eq!(cloturee, 1);
        let (e, t) = outil(&state, &jeton, "diagnostiquer_portees", json!({"salle":"mater 2"})).await;
        assert!(!e && t.contains("\"anomalies\":0"), "{t}");

        // Le sevrage passe : 500696 avec ses 10 porcelets vers PS 2.
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 2","destination":"PS 2","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré") && t.contains("10 porcelet"), "{t}");
        assert_eq!(case_pig_count(&pool, 135).await?, 10);

        // Comptage réel : 100 en PS2.
        let (e, t) = outil(&state, &jeton, "inventaire_case", json!({"salle":"PS2","nombre":100,"note":"comptage du 11/11"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("calculait 10"), "{t}");
        let (e, t) = outil(&state, &jeton, "inventaire_case", json!({"salle":"PS2","nombre":100,"note":"comptage du 11/11","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(case_pig_count(&pool, 135).await?, 100);
        let dernier: Option<i64> = sqlx::query_scalar("SELECT dernier_transfert_id FROM inventairecase ORDER BY id DESC LIMIT 1").fetch_one(&pool).await?;
        assert_eq!(dernier, sqlx::query_scalar("SELECT MAX(id) FROM transfert").fetch_one(&pool).await?);
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn connecteur_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(2).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let jeton = "a".repeat(64);
        crate::sms_free::ecrire_parametre(&pool, "mcp_jeton", &jeton).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool.clone(),
            crate::templates::build()?,
        );

        let (s, _) = rpc(&state, "mauvais", json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}})).await;
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        let (s, _) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","method":"notifications/initialized"})).await;
        assert_eq!(s, StatusCode::ACCEPTED);
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":2,"method":"tools/list"})).await;
        assert!(v["result"]["tools"].as_array().unwrap().len() >= 9);

        let (e, _) = outil(&state, &jeton, "chercher_truie", json!({"numero":"409353"})).await;
        assert!(!e);
        let (e, _) = outil(&state, &jeton, "cases_salle", json!({"salle":"verraterie"})).await;
        assert!(!e);
        let (e, _) = outil(&state, &jeton, "calendrier_ia", json!({})).await;
        assert!(!e);
        let (e, t) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"409353","salle":"Verraterie","vanne":"47"})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let avant: Option<i64> = sqlx::query_scalar("SELECT case_id FROM truie WHERE num_travail='409353'").fetch_one(&pool).await?;
        assert_ne!(avant, Some(123));
        let (e, t) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"409353","salle":"Verraterie","vanne":"47","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        let apres: Option<i64> = sqlx::query_scalar("SELECT case_id FROM truie WHERE num_travail='409353'").fetch_one(&pool).await?;
        assert_eq!(apres, Some(123));
        let (e, _) = outil(&state, &jeton, "creer_rappel", json!({"titre":"Truie 409353 : mettre sous œstrogène","date":"2026-10-29","confirme":true})).await;
        assert!(!e);
        let (_, t) = outil(&state, &jeton, "rappels", json!({})).await;
        assert!(t.contains("œstrogène"));
        let (e, _) = outil(&state, &jeton, "retenir", json!({"texte":"La vanne 12 de la Verraterie est en panne"})).await;
        assert!(!e);
        let (_, t) = outil(&state, &jeton, "memoire", json!({})).await;
        assert!(t.contains("vanne 12"));
        let (_, v) = rpc(&state, &jeton, json!({"jsonrpc":"2.0","id":3,"method":"initialize","params":{}})).await;
        assert!(v["result"]["instructions"].as_str().unwrap().contains("vanne 12"));
        let (e, _) = outil(&state, &jeton, "requete", json!({"sql":"DELETE FROM truie"})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "deplacer_truie", json!({"numero":"999999","salle":"Verraterie","vanne":"47","confirme":true})).await;
        assert!(e);

        // Porcs : morts, transferts, départs (deux temps, contrôles d'effectif).
        let morts = |p: &SqlitePool| { let p = p.clone(); async move { sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM declarationmort").fetch_one(&p).await.unwrap() } };
        let transferts = |p: &SqlitePool| { let p = p.clone(); async move { sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transfert").fetch_one(&p).await.unwrap() } };
        let (m0, t0) = (morts(&pool).await, transferts(&pool).await);
        let (e, t) = outil(&state, &jeton, "declarer_mort", json!({"salle":"Engraissement 5","nombre":1,"cause":"respiratoire"})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        assert_eq!(morts(&pool).await, m0);
        let (e, t) = outil(&state, &jeton, "declarer_mort", json!({"salle":"Engraissement 5","nombre":1,"cause":"respiratoire","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        assert_eq!(morts(&pool).await, m0 + 1);
        for args in [json!({"salle":"Engraissement 5","nombre":0,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":10000,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":1,"cause":"grippe","confirme":true}),
                     json!({"salle":"Nulle part","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Mater 1","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement","nombre":1,"cause":"Respiratoire","confirme":true}),
                     json!({"salle":"Engraissement 5","nombre":1,"cause":"Respiratoire","date":"2099-01-01","confirme":true})] {
            let (e, _) = outil(&state, &jeton, "declarer_mort", args).await;
            assert!(e);
        }
        assert_eq!(morts(&pool).await, m0 + 1);
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5,"bande":"ZZZ","confirme":true})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 5","nombre":5,"confirme":true})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"35G6K","nombre":5,"confirme":true})).await;
        assert!(e, "deux cases d'arrivée : il faut préciser");
        assert_eq!(transferts(&pool).await, t0);
        let (e, t) = outil(&state, &jeton, "transferer_porcs", json!({"salle_source":"Engraissement 5","salle_destination":"Engraissement 4","nombre":5,"confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        assert_eq!(transferts(&pool).await, t0 + 1);
        let (e, t) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":10})).await;
        assert!(!e && t.starts_with("À CONFIRMER"));
        let (e, _) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":100000,"confirme":true})).await;
        assert!(e);
        let (e, t) = outil(&state, &jeton, "depart_porcs", json!({"salle":"35G6K","nombre":10,"confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"));
        let n: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(nombre),0) FROM transfert WHERE case_dest_id IS NULL AND note LIKE 'Départ du%' AND id>(SELECT MAX(id)-5 FROM transfert)").fetch_one(&pool).await?;
        assert_eq!(n, 10);
        Ok(())
    }

    /// `EO_TEST_BASE=/var/tmp/copie.db cargo test sevrage_salle_sur_copie -- --ignored --nocapture`
    /// (la copie est modifiée : la Mater 1 est sevrée).
    #[tokio::test]
    #[ignore]
    async fn sevrage_salle_sur_copie() -> anyhow::Result<()> {
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(2).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let jeton = "a".repeat(64);
        crate::sms_free::ecrire_parametre(&pool, "mcp_jeton", &jeton).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool.clone(),
            crate::templates::build()?,
        );
        let ouvertes = |p: &SqlitePool| { let p = p.clone(); async move {
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM portee_effectif pe JOIN truie t ON t.id=pe.truie_id JOIN casesalle c ON c.id=t.case_id WHERE c.salle_id=7 AND pe.cloturee=0").fetch_one(&p).await.unwrap() } };
        let avant = ouvertes(&pool).await;
        assert!(avant > 0);
        // Comme la semaine du sevrage : PS1 et PS 2 viennent d'être vidées.
        sqlx::query("INSERT INTO inventairecase(case_id,date,nombre) SELECT c.id,date('now','localtime'),0 FROM casesalle c WHERE c.nom IN ('PS1','PS 2')")
            .execute(&pool).await?;
        let (e, _) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"Mater 9","destination":"PS1"})).await;
        assert!(e);
        let (e, _) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"Mater 1","destination":"Engraissement 1"})).await;
        assert!(e, "la destination doit être du post-sevrage");
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 1","destination":"PS1 et PS 2"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("Verraterie"));
        assert_eq!(ouvertes(&pool).await, avant);
        let (e, t) = outil(&state, &jeton, "sevrer_salle", json!({"salle":"mater 1","destination":"PS1 et PS 2","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        assert_eq!(ouvertes(&pool).await, 0);

        // Achat à la voix : semence affectée à une bande.
        let (e, t) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"fournisseur":"LB-CIA Piétrain","quantite":80,"bande":"b3.14","date":"2025-12-08"})).await;
        assert!(!e && t.starts_with("À CONFIRMER") && t.contains("B3.14"));
        let (e, _) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"bande":"B9.99","confirme":true})).await;
        assert!(e);
        let (e, t) = outil(&state, &jeton, "enregistrer_achat", json!({"categorie":"semence","montant_ht":510,"fournisseur":"LB-CIA Piétrain","quantite":80,"bande":"B3.14","date":"2025-12-08","confirme":true})).await;
        assert!(!e && t.starts_with("Enregistré"), "{t}");
        let affecte: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM achatsemence s JOIN affectationfacturebande a ON a.categorie='semence' AND a.facture_id=s.id JOIN bande b ON b.id=a.bande_id WHERE b.code='B3.14' AND s.montant_ht=510").fetch_one(&pool).await?;
        assert!(affecte >= 1);
        let en_mater: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM truie t JOIN casesalle c ON c.id=t.case_id WHERE c.salle_id=7 AND t.reformee=0").fetch_one(&pool).await?;
        println!("truies encore rangées dans une case de la Mater 1 : {en_mater}");
        Ok(())
    }
}

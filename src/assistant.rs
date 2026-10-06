//! Assistant : questions en langage naturel sur les données de l'élevage.
//!
//! Le modèle de langue (API Claude) ne touche jamais la base lui-même : il
//! choisit parmi quelques outils, et c'est ce module qui les exécute. Tous
//! les outils sont en lecture seule — la connexion est ouverte en
//! `read_only` et seules les requêtes SELECT/WITH sont acceptées. Les tables
//! qui contiennent des secrets (comptes, paramètres, accès) sont refusées.
//!
//! Réglages dans `parametre` : `assistant_cle` (clé API), `assistant_modele`.

use serde::Serialize;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqliteRow};
use sqlx::{Column, ConnectOptions, Row, SqliteConnection, SqlitePool, TypeInfo, ValueRef};
use std::path::Path;
use std::time::Duration;

pub const MODELE_DEFAUT: &str = "claude-haiku-4-5-20251001";
const API: &str = "https://api.anthropic.com/v1/messages";
/// Tours outil ↔ modèle au plus par question.
const TOURS_MAX: usize = 10;
const LIGNES_MAX: usize = 200;
const CELLULE_MAX: usize = 300;

/// Tables jamais lisibles par l'assistant : mots de passe, jetons, clés.
const TABLES_INTERDITES: [&str; 5] = [
    "utilisateur",
    "parametre",
    "sessionventedirecte",
    "reglagecommunicationventedirecte",
    "sms_envoi",
];

/// Une étape visible par l'éleveur : quel outil, avec quoi, et le résultat.
#[derive(Debug, Clone, Serialize)]
pub struct Etape {
    pub outil: String,
    pub entree: String,
    pub resume: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reponse {
    pub texte: String,
    pub etapes: Vec<Etape>,
}

fn outils() -> Value {
    json!([
        {
            "name": "lister_tables",
            "description": "Liste les tables de la base de l'élevage avec leur nombre de lignes.",
            "input_schema": {"type": "object", "properties": {}}
        },
        {
            "name": "decrire_tables",
            "description": "Donne le schéma (CREATE TABLE) des tables demandées. À utiliser avant d'écrire une requête.",
            "input_schema": {"type": "object", "properties": {
                "tables": {"type": "array", "items": {"type": "string"}}
            }, "required": ["tables"]}
        },
        {
            "name": "requete",
            "description": "Exécute UNE requête SQLite en lecture seule (SELECT ou WITH) et renvoie au plus 200 lignes en JSON. Toujours ajouter un LIMIT raisonnable.",
            "input_schema": {"type": "object", "properties": {
                "sql": {"type": "string"}
            }, "required": ["sql"]}
        }
    ])
}

fn consigne(aujourdhui: &str) -> String {
    format!(
        "Tu es l'assistant d'EO_Sevel, logiciel de gestion d'élevage. Tu réponds en français, \
         brièvement, à partir des données de la base SQLite de l'élevage, consultée uniquement \
         par tes outils (lecture seule). Nous sommes le {aujourdhui}. \
         Méthode : lister ou décrire les tables utiles, puis interroger. Les dates sont souvent \
         en texte ISO. N'invente jamais un chiffre : si les données ne permettent pas de \
         répondre, dis-le et indique ce qui manque. Tu ne peux rien modifier ; si on te \
         demande une saisie, explique où la faire dans l'application. Donne les nombres avec \
         leur unité et précise la période ou le périmètre utilisé."
    )
}

/// Refuse tout ce qui n'est pas une seule lecture, et les tables à secrets.
pub fn verifier_sql(sql: &str) -> Result<String, String> {
    let s = sql.trim().trim_end_matches(';').trim();
    if s.contains(';') {
        return Err("Une seule requête à la fois.".into());
    }
    let bas = s.to_lowercase();
    if !(bas.starts_with("select") || bas.starts_with("with")) {
        return Err("Seules les requêtes SELECT ou WITH sont permises.".into());
    }
    let mots: Vec<&str> = bas
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|m| !m.is_empty())
        .collect();
    for interdit in ["insert", "update", "delete", "drop", "alter", "attach", "pragma", "vacuum", "replace", "create"] {
        if mots.contains(&interdit) {
            return Err(format!("Mot « {interdit} » refusé : lecture seule."));
        }
    }
    for t in TABLES_INTERDITES {
        if mots.contains(&t) {
            return Err(format!("La table « {t} » n'est pas accessible à l'assistant."));
        }
    }
    Ok(s.to_string())
}

fn cellule(row: &SqliteRow, i: usize) -> Value {
    let Ok(brut) = row.try_get_raw(i) else { return Value::Null };
    if brut.is_null() {
        return Value::Null;
    }
    match brut.type_info().name() {
        "INTEGER" | "BOOLEAN" => row.try_get::<i64, _>(i).map(Value::from).unwrap_or(Value::Null),
        "REAL" => row.try_get::<f64, _>(i).map(Value::from).unwrap_or(Value::Null),
        "BLOB" => json!("<binaire>"),
        _ => row
            .try_get::<String, _>(i)
            .map(|t| {
                if t.chars().count() > CELLULE_MAX {
                    Value::from(format!("{}…", t.chars().take(CELLULE_MAX).collect::<String>()))
                } else {
                    Value::from(t)
                }
            })
            .unwrap_or(Value::Null),
    }
}

pub(crate) async fn connexion(base: &Path) -> Result<SqliteConnection, String> {
    SqliteConnectOptions::new()
        .filename(base)
        .read_only(true)
        .busy_timeout(Duration::from_secs(3))
        .connect()
        .await
        .map_err(|e| format!("Base illisible : {e}"))
}

pub(crate) async fn executer(cnx: &mut SqliteConnection, nom: &str, entree: &Value) -> (String, String) {
    match nom {
        "lister_tables" => {
            let noms: Vec<String> = sqlx::query_scalar(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .fetch_all(&mut *cnx)
            .await
            .unwrap_or_default();
            let mut liste = Vec::new();
            for n in noms.into_iter().filter(|n| !TABLES_INTERDITES.contains(&n.as_str())) {
                let nb: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM \"{n}\""))
                    .fetch_one(&mut *cnx)
                    .await
                    .unwrap_or(-1);
                liste.push(format!("{n} ({nb})"));
            }
            (format!("{} tables", liste.len()), liste.join(", "))
        }
        "decrire_tables" => {
            let tables: Vec<String> = entree["tables"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|t| t.as_str().map(str::to_string))
                .filter(|t| !TABLES_INTERDITES.contains(&t.as_str()))
                .take(15)
                .collect();
            let mut sortie = Vec::new();
            for t in &tables {
                let sql: Option<String> =
                    sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='table' AND name=?")
                        .bind(t)
                        .fetch_optional(&mut *cnx)
                        .await
                        .ok()
                        .flatten();
                sortie.push(sql.unwrap_or_else(|| format!("-- table {t} inconnue")));
            }
            (tables.join(", "), sortie.join("\n\n"))
        }
        "requete" => {
            let sql = entree["sql"].as_str().unwrap_or("");
            let sql = match verifier_sql(sql) {
                Ok(s) => s,
                Err(e) => return (format!("refusée : {e}"), format!("ERREUR : {e}")),
            };
            let lignes = tokio::time::timeout(Duration::from_secs(10), sqlx::query(&sql).fetch_all(&mut *cnx)).await;
            match lignes {
                Err(_) => ("trop longue".into(), "ERREUR : requête trop longue (10 s)".into()),
                Ok(Err(e)) => (format!("erreur : {e}"), format!("ERREUR SQL : {e}")),
                Ok(Ok(rows)) => {
                    let total = rows.len();
                    let colonnes: Vec<String> = rows
                        .first()
                        .map(|r| r.columns().iter().map(|c| c.name().to_string()).collect())
                        .unwrap_or_default();
                    let donnees: Vec<Value> = rows
                        .iter()
                        .take(LIGNES_MAX)
                        .map(|r| Value::Array((0..colonnes.len()).map(|i| cellule(r, i)).collect()))
                        .collect();
                    let tronque = total > LIGNES_MAX;
                    (
                        format!("{total} ligne{}", if total > 1 { "s" } else { "" }),
                        json!({"colonnes": colonnes, "lignes": donnees, "tronque": tronque}).to_string(),
                    )
                }
            }
        }
        autre => (format!("outil inconnu {autre}"), format!("ERREUR : outil inconnu {autre}")),
    }
}

/// Pose la question au modèle et fait tourner la boucle d'outils.
pub async fn demander(pool: &SqlitePool, base: &Path, question: &str) -> Result<Reponse, String> {
    let cle = crate::proxmox::parametre(pool, "assistant_cle")
        .await
        .ok_or("Clé API non enregistrée (Mes équipements → Assistant → Réglages).")?;
    let modele = crate::proxmox::parametre(pool, "assistant_modele")
        .await
        .unwrap_or_else(|| MODELE_DEFAUT.to_string());
    let aujourdhui = chrono::Local::now().format("%A %d/%m/%Y").to_string();
    let mut cnx = connexion(base).await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| e.to_string())?;

    let mut messages = vec![json!({"role": "user", "content": question})];
    let mut etapes = Vec::new();
    for _ in 0..TOURS_MAX {
        let corps = json!({
            "model": modele,
            "max_tokens": 2048,
            "system": [{"type": "text", "text": consigne(&aujourdhui), "cache_control": {"type": "ephemeral"}}],
            "tools": outils(),
            "messages": messages,
        });
        let r = client
            .post(API)
            .header("x-api-key", &cle)
            .header("anthropic-version", "2023-06-01")
            .json(&corps)
            .send()
            .await
            .map_err(|e| format!("API injoignable : {}", e.without_url()))?;
        let statut = r.status();
        let v: Value = r.json().await.map_err(|e| format!("Réponse illisible : {e}"))?;
        if !statut.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("erreur inconnue");
            return Err(match statut.as_u16() {
                401 => "Clé API refusée.".to_string(),
                c => format!("API ({c}) : {msg}"),
            });
        }
        let contenu = v["content"].as_array().cloned().unwrap_or_default();
        let appels: Vec<&Value> = contenu.iter().filter(|b| b["type"] == "tool_use").collect();
        if v["stop_reason"] != "tool_use" || appels.is_empty() {
            let texte = contenu
                .iter()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            tracing::info!(question, outils = etapes.len(), "assistant : réponse");
            return Ok(Reponse { texte, etapes });
        }
        let mut resultats = Vec::new();
        for appel in appels {
            let nom = appel["name"].as_str().unwrap_or("");
            let entree = &appel["input"];
            let (resume, resultat) = executer(&mut cnx, nom, entree).await;
            etapes.push(Etape {
                outil: nom.to_string(),
                entree: entree["sql"]
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| if entree.as_object().is_some_and(|o| o.is_empty()) { String::new() } else { entree.to_string() }),
                resume,
            });
            resultats.push(json!({"type": "tool_result", "tool_use_id": appel["id"], "content": resultat}));
        }
        messages.push(json!({"role": "assistant", "content": contenu}));
        messages.push(json!({"role": "user", "content": resultats}));
    }
    Err("Trop d'étapes sans réponse : reformulez la question plus précisément.".into())
}

#[cfg(test)]
mod tests {
    use super::verifier_sql;

    #[test]
    fn lecture_seule() {
        assert!(verifier_sql("SELECT COUNT(*) FROM truie").is_ok());
        assert!(verifier_sql("with x as (select 1) select * from x;").is_ok());
        assert!(verifier_sql("DELETE FROM truie").is_err());
        assert!(verifier_sql("SELECT 1; DROP TABLE truie").is_err());
        assert!(verifier_sql("SELECT * FROM utilisateur").is_err());
        assert!(verifier_sql("select valeur from parametre where cle='x'").is_err());
        assert!(verifier_sql("PRAGMA table_info(truie)").is_err());
    }
}

/// `EO_TEST_BASE=/var/tmp/copie.db cargo test assistant_sur_copie -- --ignored`
#[cfg(test)]
mod tests_copie {
    use super::*;

    #[tokio::test]
    #[ignore]
    async fn assistant_sur_copie() -> anyhow::Result<()> {
        let base = std::path::PathBuf::from(std::env::var("EO_TEST_BASE")?);
        let mut cnx = connexion(&base).await.map_err(anyhow::Error::msg)?;
        let (r, tables) = executer(&mut cnx, "lister_tables", &json!({})).await;
        println!("{r}");
        assert!(tables.contains("truie (") && !tables.contains("utilisateur"));
        let (_, schema) = executer(&mut cnx, "decrire_tables", &json!({"tables": ["truie", "parametre"]})).await;
        assert!(schema.contains("CREATE TABLE") && !schema.contains("valeur"));
        let (r, l) = executer(&mut cnx, "requete", &json!({"sql": "SELECT COUNT(*) AS n, 1.5 AS x, 'a' AS t, NULL AS z FROM truie"})).await;
        println!("{r} {l}");
        assert!(l.contains("\"colonnes\":[\"n\",\"x\",\"t\",\"z\"]"));
        let (r, _) = executer(&mut cnx, "requete", &json!({"sql": "DELETE FROM truie"})).await;
        assert!(r.starts_with("refusée"));
        // Écriture refusée par la connexion elle-même, même si le filtre laissait passer.
        assert!(sqlx::query("CREATE TABLE essai_ecriture(x)").execute(&mut cnx).await.is_err());

        let env = crate::templates::build()?;
        let page = env.get_template("assistant.html")?.render(json!({
            "session": {"est_admin": true, "csrf": "x", "pages": []},
            "cle_enregistree": true, "question": "Combien de truies ?",
            "reponse": {"texte": "Il y a 12 truies.", "etapes": [{"outil": "requete", "entree": "SELECT 1", "resume": "1 ligne"}]}
        }))?;
        assert!(page.contains("Il y a 12 truies.") && page.contains("SELECT 1"));
        Ok(())
    }
}

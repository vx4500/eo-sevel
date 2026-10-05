//! Mes équipements → Serveur Proxmox : lecture seule de l'hôte qui fait
//! tourner EO_Sevel (processeur, mémoire, disques, machines et conteneurs,
//! sauvegardes) par l'API de Proxmox, avec un jeton en lecture seule
//! (rôle PVEAuditor). Alertes par SMS via le module Free SMS.

use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::time::Duration;

pub async fn parametre(pool: &SqlitePool, cle: &str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>("SELECT valeur FROM parametre WHERE cle=?")
        .bind(cle)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .flatten()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Adresse normalisée : « 192.168.1.10 » → « https://192.168.1.10:8006 ».
pub fn normaliser_adresse(brute: &str) -> String {
    let mut a = brute.trim().trim_end_matches('/').to_string();
    if !a.starts_with("http://") && !a.starts_with("https://") {
        a = format!("https://{a}");
    }
    let hote = a.splitn(2, "://").nth(1).unwrap_or("");
    if !hote.contains(':') {
        a.push_str(":8006");
    }
    a
}

struct Acces {
    url: String,
    jeton: String,
    secret: String,
}

async fn acces(pool: &SqlitePool) -> Option<Acces> {
    Some(Acces {
        url: parametre(pool, "proxmox_url").await?,
        jeton: parametre(pool, "proxmox_jeton").await?,
        secret: parametre(pool, "proxmox_secret").await?,
    })
}

async fn get(client: &reqwest::Client, a: &Acces, chemin: &str) -> Result<Value, String> {
    let r = client
        .get(format!("{}/api2/json{chemin}", a.url))
        .header("Authorization", format!("PVEAPIToken={}={}", a.jeton, a.secret))
        .send()
        .await
        .map_err(|e| format!("Proxmox injoignable : {}", e.without_url()))?;
    match r.status().as_u16() {
        200 => {}
        401 => return Err("Jeton refusé : vérifiez l'identifiant (utilisateur@pam!nom) et le secret".into()),
        403 => return Err("Droits insuffisants : donnez le rôle PVEAuditor au jeton sur « / »".into()),
        c => return Err(format!("Réponse inattendue de Proxmox ({c})")),
    }
    let v: Value = r.json().await.map_err(|e| format!("Réponse illisible : {e}"))?;
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

fn f(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

fn pct(used: f64, total: f64) -> Option<f64> {
    (total > 0.0).then(|| (used / total * 1000.0).round() / 10.0)
}

/// « 1,5 Go », « 820 Mo ».
pub fn taille(octets: f64) -> String {
    let go = octets / 1_073_741_824.0;
    if go >= 1000.0 {
        format!("{:.1} To", go / 1024.0).replace('.', ",")
    } else if go >= 1.0 {
        format!("{go:.1} Go").replace('.', ",")
    } else {
        format!("{:.0} Mo", octets / 1_048_576.0)
    }
}

/// « 12 j 4 h », « 3 h 05 ».
pub fn depuis(secondes: f64) -> String {
    let s = secondes as i64;
    if s <= 0 {
        return "—".into();
    }
    let (j, h, m) = (s / 86400, (s % 86400) / 3600, (s % 3600) / 60);
    if j > 0 {
        format!("{j} j {h} h")
    } else {
        format!("{h} h {m:02}")
    }
}

fn date(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%d/%m à %H:%M").to_string())
        .unwrap_or_else(|| "?".into())
}

/// Lecture complète, mise en forme pour la page et les alertes.
pub async fn lire(pool: &SqlitePool) -> Result<Value, String> {
    let a = acces(pool).await.ok_or("Adresse et jeton à renseigner")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // Proxmox utilise par défaut un certificat auto-signé, sur le réseau local.
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| e.to_string())?;
    let maintenant = chrono::Utc::now().timestamp();
    let mut noeuds = Vec::new();
    let liste = get(&client, &a, "/nodes").await?;
    for n in liste.as_array().cloned().unwrap_or_default() {
        let nom = n.get("node").and_then(Value::as_str).unwrap_or("").to_string();
        let en_ligne = n.get("status").and_then(Value::as_str) == Some("online");
        let mut noeud = json!({"nom": nom, "en_ligne": en_ligne});
        if en_ligne {
            let st = get(&client, &a, &format!("/nodes/{nom}/status")).await.unwrap_or(Value::Null);
            let mem = st.get("memory").cloned().unwrap_or(Value::Null);
            let root = st.get("rootfs").cloned().unwrap_or(Value::Null);
            noeud["cpu"] = json!((f(&st, "cpu") * 1000.0).round() / 10.0);
            noeud["coeurs"] = json!(st.pointer("/cpuinfo/cpus").and_then(Value::as_i64));
            noeud["modele"] = json!(st.pointer("/cpuinfo/model").and_then(Value::as_str));
            noeud["charge"] = json!(st.get("loadavg").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" · ")));
            noeud["mem_pct"] = json!(pct(f(&mem, "used"), f(&mem, "total")));
            noeud["mem_txt"] = json!(format!("{} / {}", taille(f(&mem, "used")), taille(f(&mem, "total"))));
            noeud["root_pct"] = json!(pct(f(&root, "used"), f(&root, "total")));
            noeud["root_txt"] = json!(format!("{} / {}", taille(f(&root, "used")), taille(f(&root, "total"))));
            noeud["uptime"] = json!(depuis(f(&st, "uptime")));
            noeud["version"] = json!(st.get("pveversion").and_then(Value::as_str));

            let mut invites = Vec::new();
            for (genre, libelle) in [("qemu", "VM"), ("lxc", "Conteneur")] {
                let l = get(&client, &a, &format!("/nodes/{nom}/{genre}")).await.unwrap_or(Value::Null);
                for g in l.as_array().cloned().unwrap_or_default() {
                    let marche = g.get("status").and_then(Value::as_str) == Some("running");
                    invites.push(json!({
                        "vmid": g.get("vmid").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))),
                        "nom": g.get("name").and_then(Value::as_str).unwrap_or("?"),
                        "genre": libelle, "marche": marche,
                        "cpu": (f(&g, "cpu") * 1000.0).round() / 10.0,
                        "mem_pct": pct(f(&g, "mem"), f(&g, "maxmem")),
                        "mem_txt": format!("{} / {}", taille(f(&g, "mem")), taille(f(&g, "maxmem"))),
                        "uptime": depuis(f(&g, "uptime")),
                    }));
                }
            }
            invites.sort_by_key(|g| g["vmid"].as_i64().unwrap_or(0));
            noeud["invites"] = Value::Array(invites);

            let s = get(&client, &a, &format!("/nodes/{nom}/storage")).await.unwrap_or(Value::Null);
            let stockages: Vec<Value> = s
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|s| f(s, "total") > 0.0)
                .map(|s| json!({
                    "nom": s.get("storage").and_then(Value::as_str).unwrap_or("?"),
                    "type": s.get("type").and_then(Value::as_str).unwrap_or(""),
                    "pct": pct(f(&s, "used"), f(&s, "total")),
                    "txt": format!("{} / {}", taille(f(&s, "used")), taille(f(&s, "total"))),
                }))
                .collect();
            noeud["stockages"] = Value::Array(stockages);

            let t = get(&client, &a, &format!("/nodes/{nom}/tasks?typefilter=vzdump&limit=30")).await.unwrap_or(Value::Null);
            let taches = t.as_array().cloned().unwrap_or_default();
            let derniere_ok = taches
                .iter()
                .filter(|t| t.get("status").and_then(Value::as_str) == Some("OK"))
                .filter_map(|t| t.get("endtime").and_then(Value::as_i64))
                .max();
            let derniere = taches.iter().max_by_key(|t| t.get("starttime").and_then(Value::as_i64).unwrap_or(0));
            noeud["sauvegarde"] = json!({
                "existe": !taches.is_empty(),
                "derniere_ok": derniere_ok.map(date),
                "age_h": derniere_ok.map(|d| (maintenant - d) / 3600),
                "dernier_statut": derniere.and_then(|t| t.get("status").and_then(Value::as_str)),
                "derniere_date": derniere.and_then(|t| t.get("starttime").and_then(Value::as_i64)).map(date),
            });
        }
        noeuds.push(noeud);
    }
    Ok(json!({"noeuds": noeuds, "lu_a": chrono::Local::now().format("%H:%M:%S").to_string()}))
}

/// Problèmes à signaler : (clé stable, texte).
pub fn problemes(etat: &Value, invites_suivis: &[i64]) -> Vec<(String, String)> {
    let mut p = Vec::new();
    for n in etat["noeuds"].as_array().cloned().unwrap_or_default() {
        let nom = n["nom"].as_str().unwrap_or("?");
        if !n["en_ligne"].as_bool().unwrap_or(false) {
            p.push((format!("noeud:{nom}"), format!("serveur {nom} hors ligne")));
            continue;
        }
        if n["mem_pct"].as_f64().is_some_and(|v| v >= 95.0) {
            p.push((format!("mem:{nom}"), format!("mémoire de {nom} saturée ({} %)", n["mem_pct"])));
        }
        if n["root_pct"].as_f64().is_some_and(|v| v >= 90.0) {
            p.push((format!("root:{nom}"), format!("disque système de {nom} plein à {} %", n["root_pct"])));
        }
        for s in n["stockages"].as_array().cloned().unwrap_or_default() {
            if s["pct"].as_f64().is_some_and(|v| v >= 90.0) {
                p.push((format!("sto:{nom}:{}", s["nom"].as_str().unwrap_or("")), format!("stockage {} plein à {} %", s["nom"].as_str().unwrap_or("?"), s["pct"])));
            }
        }
        for g in n["invites"].as_array().cloned().unwrap_or_default() {
            let id = g["vmid"].as_i64().unwrap_or(0);
            if invites_suivis.contains(&id) && !g["marche"].as_bool().unwrap_or(false) {
                p.push((format!("vm:{id}"), format!("{} {} ({id}) arrêté", g["genre"].as_str().unwrap_or(""), g["nom"].as_str().unwrap_or("?"))));
            }
        }
        let sv = &n["sauvegarde"];
        if sv["existe"].as_bool().unwrap_or(false) && sv["age_h"].as_i64().is_none_or(|h| h > 48) {
            p.push((format!("sauv:{nom}"), format!("aucune sauvegarde réussie sur {nom} depuis plus de 48 h")));
        }
    }
    p
}

/// Machines et conteneurs dont l'arrêt déclenche une alerte : ceux qui
/// tournaient lors de l'enregistrement des réglages.
pub async fn invites_suivis(pool: &SqlitePool) -> Vec<i64> {
    parametre(pool, "proxmox_suivis")
        .await
        .map(|v| v.split(',').filter_map(|s| s.trim().parse().ok()).collect())
        .unwrap_or_default()
}

async fn tour(pool: &SqlitePool) -> anyhow::Result<()> {
    if parametre(pool, "proxmox_alertes").await.as_deref() != Some("1") {
        return Ok(());
    }
    let etat = match lire(pool).await {
        Ok(e) => e,
        Err(_) => json!({"noeuds": [{"nom": "Proxmox", "en_ligne": false}]}),
    };
    let actuels = problemes(&etat, &invites_suivis(pool).await);
    let deja: Vec<String> = parametre(pool, "proxmox_signales")
        .await
        .map(|v| v.split('|').map(str::to_string).collect())
        .unwrap_or_default();
    let nouveaux: Vec<&(String, String)> = actuels.iter().filter(|(k, _)| !deja.contains(k)).collect();
    if !nouveaux.is_empty() && parametre(pool, "sms_free_actif").await.as_deref() == Some("1") {
        let mut texte = String::from("EO_Sevel SERVEUR");
        for (_, t) in nouveaux.iter().take(5) {
            texte.push_str(&format!("\n- {t}"));
        }
        if crate::sms_free::envoyer(pool, &texte, "proxmox").await.is_err() {
            return Ok(()); // réessai au tour suivant
        }
    }
    let cles: Vec<&str> = actuels.iter().map(|(k, _)| k.as_str()).collect();
    crate::sms_free::ecrire_parametre(pool, "proxmox_signales", &cles.join("|")).await?;
    Ok(())
}

/// Boucle lancée au démarrage d'eo-sevel : contrôle toutes les 5 minutes.
pub async fn boucle(pool: SqlitePool) {
    tokio::time::sleep(Duration::from_secs(90)).await;
    loop {
        if let Err(e) = tour(&pool).await {
            tracing::error!(error=%e, "Boucle Proxmox");
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mise_en_forme_et_problemes() {
        assert_eq!(normaliser_adresse("192.168.1.10"), "https://192.168.1.10:8006");
        assert_eq!(normaliser_adresse("https://pve:8006/"), "https://pve:8006");
        assert_eq!(taille(1_610_612_736.0), "1,5 Go");
        assert_eq!(depuis(90_000.0), "1 j 1 h");
        let etat = json!({"noeuds": [{"nom": "pve", "en_ligne": true, "mem_pct": 50.0, "root_pct": 93.0,
            "stockages": [{"nom": "local-lvm", "pct": 40.0}],
            "invites": [{"vmid": 101, "nom": "eo-sevel", "genre": "Conteneur", "marche": false}, {"vmid": 102, "nom": "test", "genre": "VM", "marche": false}],
            "sauvegarde": {"existe": true, "age_h": 20}}]});
        let p = problemes(&etat, &[101]);
        let cles: Vec<&str> = p.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(cles, vec!["root:pve", "vm:101"]);
    }
}

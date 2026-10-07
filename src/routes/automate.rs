//! Vue d'ensemble de l'automate IPX800 (/ambiance/automate) : l'état de
//! l'accès, de la régulation et de l'inventaire en un coup d'œil, et les
//! étapes de mise en route qui restent à faire. Rien n'est lu en direct sur
//! l'automate : la page s'affiche instantanément même s'il ne répond pas.

use super::*;

async fn compter(pool: &SqlitePool, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(pool).await.unwrap_or(0)
}

pub(super) async fn page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let hote = super::ambiance::parametre_texte(pool, "ambiance_ipx_hote", "").await;
    let cle_definie = !super::ambiance::parametre_texte(pool, "ambiance_ipx_cle", "")
        .await
        .is_empty();
    let demon = super::ambiance::etat_demon(pool).await;

    let nb_points = compter(pool, "SELECT COUNT(*) FROM ipx_point WHERE actif=1").await;
    let nb_pilotables = compter(
        pool,
        "SELECT COUNT(*) FROM ipx_point WHERE actif=1 AND pilotable=1 \
         AND chemin_ecriture IS NOT NULL AND trim(chemin_ecriture)<>''",
    )
    .await;
    let relais_eclairage = compter(
        pool,
        "SELECT COUNT(*) FROM ipx_point WHERE actif=1 AND pilotable=1 AND role='eclairage' \
         AND chemin_ecriture IS NOT NULL AND trim(chemin_ecriture)<>''",
    )
    .await;
    let mesurees = compter(pool, "SELECT COUNT(*) FROM ambiance_salle WHERE actif=1").await;
    let pilotees =
        compter(pool, "SELECT COUNT(*) FROM ambiance_salle WHERE actif=1 AND pilote=1").await;
    let programmes = compter(pool, "SELECT COUNT(*) FROM eclairage_programme").await;
    let en_service = compter(
        pool,
        "SELECT COUNT(*) FROM eclairage_programme WHERE actif=1 AND essai=0",
    )
    .await;
    let essai = compter(
        pool,
        "SELECT COUNT(*) FROM eclairage_programme WHERE actif=1 AND essai=1",
    )
    .await;
    let alertes = compter(pool, "SELECT COUNT(*) FROM ambiance_alerte WHERE fin IS NULL").await;

    let comptes: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT role, COUNT(*), SUM(pilotable) FROM ipx_point WHERE actif=1 GROUP BY role",
    )
    .fetch_all(pool)
    .await?;
    let roles: Vec<Value> = super::ambiance::ROLES
        .iter()
        .filter_map(|(code, libelle, _)| {
            comptes
                .iter()
                .find(|(role, _, _)| role == code)
                .map(|(_, nombre, pilotables)| {
                    json!({"libelle": libelle, "nombre": nombre, "pilotables": pilotables})
                })
        })
        .collect();

    let etapes = [
        (!hote.is_empty(), "Indiquer l'adresse de l'automate", "/ambiance/ipx/acces", "Accès"),
        (cle_definie, "Enregistrer la clé d'API", "/ambiance/ipx/acces", "Accès"),
        (nb_points > 0, "Nommer les entrées et sorties utiles", "/ambiance/ipx/carte", "Carte"),
        (mesurees > 0, "Raccorder les sondes aux salles", "/ambiance", "Ambiance"),
        (
            relais_eclairage > 0,
            "Déclarer les relais d'éclairage (rôle Éclairage, pilotable)",
            "/ambiance/ipx/carte",
            "Carte",
        ),
    ];
    let prochaine = etapes.iter().position(|(fait, ..)| !fait);
    let etapes: Vec<Value> = etapes
        .iter()
        .map(|(fait, texte, lien, action)| {
            json!({"fait": fait, "texte": texte, "lien": lien, "action": action})
        })
        .collect();

    let mut contexte = context(&session);
    contexte.insert("acces".into(), json!({"hote": hote, "cle_definie": cle_definie}));
    contexte.insert("demon".into(), demon);
    contexte.insert("nb_points".into(), json!(nb_points));
    contexte.insert("nb_pilotables".into(), json!(nb_pilotables));
    contexte.insert("salles".into(), json!({"mesurees": mesurees, "pilotees": pilotees}));
    contexte.insert(
        "eclairage".into(),
        json!({"programmes": programmes, "en_service": en_service, "essai": essai}),
    );
    contexte.insert("alertes".into(), json!(alertes));
    contexte.insert("roles".into(), Value::Array(roles));
    contexte.insert("etapes".into(), Value::Array(etapes));
    contexte.insert("prochaine".into(), json!(prochaine));
    contexte.insert(
        "commandes".into(),
        Value::Array(
            generic_rows(
                pool,
                "SELECT horodatage, utilisateur, nom_point, valeur, resultat \
                 FROM ipx_commande_journal ORDER BY id DESC LIMIT 10",
            )
            .await?,
        ),
    );
    render(&state, "ambiance_automate.html", Value::Object(contexte))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn la_vue_d_ensemble_s_affiche_sur_une_base_vierge() -> anyhow::Result<()> {
        let state = super::super::documents::tests::state().await;
        for sql in [
            include_str!("../../migrations/0007_ipx_points.sql"),
            include_str!("../../migrations/0016_eclairage.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&state.pool).await?;
        }
        sqlx::query("INSERT INTO ipx_point(hote,chemin,nom,role) VALUES('ipx-test.invalid','api/core/ana/1','Lux','luminosite')")
            .execute(&state.pool)
            .await?;
        let html = page(State(state), Extension(super::super::documents::tests::session()))
            .await?
            .0;
        assert!(html.contains("Mise en route") && html.contains("Luminosité"));
        assert!(html.contains("Indiquer l&#x27;adresse de l&#x27;automate"), "{html}");
        Ok(())
    }

    #[tokio::test]
    async fn chaque_onglet_de_l_automate_n_affiche_que_sa_partie() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let state = super::super::documents::tests::state().await;
        sqlx::raw_sql(include_str!("../../migrations/0007_ipx_points.sql"))
            .execute(&state.pool)
            .await?;
        // Colonne ajoutée par `ensure_column` en production.
        sqlx::raw_sql("ALTER TABLE ipx_point ADD COLUMN formule TEXT")
            .execute(&state.pool)
            .await?;
        let vue = |nom: &str| {
            let mut p = HashMap::new();
            p.insert("vue".to_string(), nom.to_string());
            Query(p)
        };
        let ipx = super::super::ambiance::ipx_page;
        let points = ipx(State(state.clone()), Extension(session()), vue("points")).await?.0;
        assert!(points.contains("Entrées et sorties enregistrées"));
        assert!(!points.contains("Explorateur en direct") && !points.contains("Gabarit de lecture"));
        let explorateur = ipx(State(state.clone()), Extension(session()), vue("explorateur")).await?.0;
        assert!(explorateur.contains("Explorateur en direct"));
        assert!(!explorateur.contains("Entrées et sorties enregistrées"));
        let acces = ipx(State(state.clone()), Extension(session()), vue("acces")).await?.0;
        assert!(acces.contains("Gabarit de lecture") && !acces.contains("Explorateur en direct"));
        assert!(acces.contains("class=\"actif\" aria-current=\"page\">⚙️ Accès"));
        Ok(())
    }
}

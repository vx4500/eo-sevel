//! Sessions de connexion conservées en base (table `session_web`).
//!
//! La DashMap `AppState::sessions` reste le cache rapide ; la table est la
//! référence qui survit aux mises à jour et aux redémarrages : quand un cookie
//! n'est plus en mémoire, la session est rechargée depuis la base et remise
//! dans la DashMap, avec un nouveau jeton CSRF. La règle d'inactivité de
//! `auth::INACTIVITE_MAX_S` s'applique aussi à la ligne en base, et `vu_le`
//! n'est réécrit qu'au plus une fois par minute pour ne pas charger SQLite.

use super::{module_actif, type_elevage_actif, AppState};
use crate::auth::{self, SessionData, INACTIVITE_MAX_S};
use crate::error::AppResult;
use crate::models::Utilisateur;

/// Intervalle minimal entre deux écritures de `vu_le` pour une même session.
const VU_LE_PAS_S: i64 = 60;

const COLONNES_UTILISATEUR: &str =
    "id,identifiant,nom,prenom,hash_mdp,role,actif,sections,doit_changer_mdp,tentatives_echec,bloque_jusqu";

/// Construit les données de session d'un utilisateur (même contenu qu'à la
/// connexion : nom affiché, pages, type d'élevage et modules mis en cache).
pub async fn donnees_session(state: &AppState, user: &Utilisateur) -> AppResult<SessionData> {
    let full_name = format!(
        "{} {}",
        user.prenom.clone().unwrap_or_default(),
        user.nom.clone().unwrap_or_else(|| user.identifiant.clone())
    )
    .trim()
    .to_string();
    let sections = user
        .sections
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    Ok(SessionData {
        uid: user.id,
        identifiant: user.identifiant.clone(),
        nom: full_name,
        role: user.role.clone(),
        sections,
        csrf: auth::new_csrf(),
        doit_changer_mdp: user.doit_changer_mdp,
        type_elevage: type_elevage_actif(&state.pool).await?,
        module_genetique: module_actif(&state.pool, "module_genetique", false).await?,
        module_prestataires: module_actif(&state.pool, "module_prestataires", true).await?,
        module_charcutiers_rfid: module_actif(&state.pool, "module_charcutiers_rfid", false)
            .await?,
        module_vente_directe: module_actif(&state.pool, "module_vente_directe", true).await?,
        module_equipements: module_actif(&state.pool, "module_equipements", true).await?,
    })
}

/// Ouvre une session : en mémoire et en base. Renvoie l'identifiant à poser
/// dans le cookie `eo_session`.
pub async fn ouvrir(state: &AppState, user: &Utilisateur) -> AppResult<String> {
    let session_id = auth::new_secure_token();
    let donnees = donnees_session(state, user).await?;
    let maintenant = chrono::Utc::now().timestamp();
    sqlx::query("INSERT INTO session_web(id,uid,cree_le,vu_le) VALUES(?,?,?,?)")
        .bind(&session_id)
        .bind(user.id)
        .bind(maintenant)
        .bind(maintenant)
        .execute(&state.pool)
        .await?;
    state.sessions.insert(session_id.clone(), donnees);
    state.sessions_vues.insert(session_id.clone(), maintenant);
    Ok(session_id)
}

/// Retrouve la session d'un cookie : d'abord en mémoire, sinon en base (cas
/// d'un redémarrage), où l'utilisateur est rechargé s'il est toujours actif.
/// Applique la règle d'inactivité et note le passage (`vu_le`) au plus une
/// fois par minute. `None` = cookie inconnu ou session expirée.
pub async fn retrouver(state: &AppState, session_id: &str, maintenant: i64) -> Option<SessionData> {
    let en_memoire = state
        .sessions
        .get(session_id)
        .map(|entry| entry.value().clone());
    let (session, derniere_vue) = match en_memoire {
        Some(session) => (session, state.sessions_vues.get(session_id).map(|v| *v)),
        None => {
            let ligne: Option<(i64, i64)> =
                sqlx::query_as("SELECT uid,vu_le FROM session_web WHERE id=?")
                    .bind(session_id)
                    .fetch_optional(&state.pool)
                    .await
                    .ok()
                    .flatten();
            let (uid, vu_le) = ligne?;
            if maintenant - vu_le > INACTIVITE_MAX_S {
                fermer(state, session_id).await;
                return None;
            }
            let user = sqlx::query_as::<_, Utilisateur>(&format!(
                "SELECT {COLONNES_UTILISATEUR} FROM utilisateur WHERE id=? AND actif=1 LIMIT 1"
            ))
            .bind(uid)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
            let Some(user) = user else {
                // Compte désactivé ou supprimé entre-temps : la ligne ne sert plus.
                fermer(state, session_id).await;
                return None;
            };
            let Ok(donnees) = donnees_session(state, &user).await else {
                return None;
            };
            // Un autre worker a pu recharger la même session en parallèle :
            // on garde la première version pour ne pas changer son jeton CSRF.
            let donnees = state
                .sessions
                .entry(session_id.to_string())
                .or_insert(donnees)
                .value()
                .clone();
            (donnees, Some(vu_le))
        }
    };
    if derniere_vue.is_some_and(|vue| maintenant - vue > INACTIVITE_MAX_S) {
        fermer(state, session_id).await;
        return None;
    }
    if derniere_vue.is_none_or(|vue| maintenant - vue >= VU_LE_PAS_S) {
        state
            .sessions_vues
            .insert(session_id.to_string(), maintenant);
        if let Err(e) = sqlx::query("UPDATE session_web SET vu_le=? WHERE id=?")
            .bind(maintenant)
            .bind(session_id)
            .execute(&state.pool)
            .await
        {
            tracing::warn!(error=%e, "session_web : horodatage non mis à jour");
        }
    }
    Some(session)
}

/// Ferme une session (déconnexion) : mémoire et base.
pub async fn fermer(state: &AppState, session_id: &str) {
    state.sessions.remove(session_id);
    state.sessions_vues.remove(session_id);
    if let Err(e) = sqlx::query("DELETE FROM session_web WHERE id=?")
        .bind(session_id)
        .execute(&state.pool)
        .await
    {
        tracing::warn!(error=%e, "session_web : suppression impossible");
    }
}

/// Ferme toutes les sessions d'un utilisateur (changement de mot de passe,
/// de droits, désactivation du compte) : mémoire et base.
pub async fn fermer_utilisateur(state: &AppState, uid: i64) {
    let ids: Vec<String> = state
        .sessions
        .iter()
        .filter(|entry| entry.value().uid == uid)
        .map(|entry| entry.key().clone())
        .collect();
    for id in ids {
        state.sessions.remove(&id);
        state.sessions_vues.remove(&id);
    }
    if let Err(e) = sqlx::query("DELETE FROM session_web WHERE uid=?")
        .bind(uid)
        .execute(&state.pool)
        .await
    {
        tracing::warn!(error=%e, "session_web : suppression par utilisateur impossible");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    async fn etat() -> anyhow::Result<AppState> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await?;
        // Le schéma complet exige le mode WAL (fichier) : seules les tables
        // utiles au test sont créées ici.
        for migration in [
            include_str!("../../migrations/0001_schema.sql"),
            include_str!("../../migrations/0033_session_web.sql"),
        ] {
            sqlx::raw_sql(migration).execute(&pool).await?;
        }
        sqlx::query("INSERT INTO utilisateur(id,identifiant,nom,prenom,hash_mdp,role,actif,sections) VALUES(7,'marie','Dupont','Marie','x$y','eleveur',1,'planning,bandes')")
            .execute(&pool)
            .await?;
        Ok(AppState::new(
            Config {
                bind: "127.0.0.1:8080".parse()?,
                db_path: ":memory:".into(),
                secure_cookies: false,
            },
            pool,
            crate::templates::build()?,
        ))
    }

    async fn utilisateur(state: &AppState) -> anyhow::Result<Utilisateur> {
        Ok(sqlx::query_as::<_, Utilisateur>(&format!(
            "SELECT {COLONNES_UTILISATEUR} FROM utilisateur WHERE id=7"
        ))
        .fetch_one(&state.pool)
        .await?)
    }

    async fn lignes(state: &AppState) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM session_web")
            .fetch_one(&state.pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn session_survit_a_un_redemarrage() -> anyhow::Result<()> {
        let state = etat().await?;
        let user = utilisateur(&state).await?;
        let sid = ouvrir(&state, &user).await?;
        assert_eq!(lignes(&state).await, 1);
        let avant = retrouver(&state, &sid, 1_000).await.expect("en mémoire");
        assert_eq!(avant.uid, 7);

        // Redémarrage simulé : la mémoire est vide, la base reste.
        state.sessions.clear();
        state.sessions_vues.clear();
        let apres = retrouver(&state, &sid, 1_000).await.expect("rechargée depuis la base");
        assert_eq!(apres.uid, 7);
        assert_eq!(apres.identifiant, "marie");
        assert_eq!(apres.nom, "Marie Dupont");
        assert_eq!(apres.sections, vec!["planning".to_string(), "bandes".to_string()]);
        assert_ne!(apres.csrf, avant.csrf, "nouveau jeton CSRF après rechargement");
        assert!(state.sessions.contains_key(&sid), "remise en mémoire");

        // Cookie inconnu : rien.
        assert!(retrouver(&state, "inconnu", 1_000).await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_deconnexion_invalide_le_cookie() -> anyhow::Result<()> {
        let state = etat().await?;
        let user = utilisateur(&state).await?;
        let sid = ouvrir(&state, &user).await?;
        fermer(&state, &sid).await;
        assert_eq!(lignes(&state).await, 0);
        assert!(!state.sessions.contains_key(&sid));
        state.sessions.clear();
        assert!(retrouver(&state, &sid, 1_000).await.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn session_inactive_trente_jours_est_fermee_en_base() -> anyhow::Result<()> {
        let state = etat().await?;
        let user = utilisateur(&state).await?;
        let sid = ouvrir(&state, &user).await?;
        let maintenant = chrono::Utc::now().timestamp();
        state.sessions.clear();
        state.sessions_vues.clear();
        assert!(retrouver(&state, &sid, maintenant + INACTIVITE_MAX_S + 1).await.is_none());
        assert_eq!(lignes(&state).await, 0);
        Ok(())
    }

    #[tokio::test]
    async fn session_vu_le_ecrit_au_plus_une_fois_par_minute() -> anyhow::Result<()> {
        let state = etat().await?;
        let user = utilisateur(&state).await?;
        let sid = ouvrir(&state, &user).await?;
        let vu = |state: &AppState| {
            let pool = state.pool.clone();
            let sid = sid.clone();
            async move {
                sqlx::query_scalar::<_, i64>("SELECT vu_le FROM session_web WHERE id=?")
                    .bind(sid)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        let depart = vu(&state).await;
        retrouver(&state, &sid, depart + 10).await.unwrap();
        assert_eq!(vu(&state).await, depart, "moins d'une minute : pas d'écriture");
        retrouver(&state, &sid, depart + 61).await.unwrap();
        assert_eq!(vu(&state).await, depart + 61);
        Ok(())
    }

    #[tokio::test]
    async fn session_compte_desactive_ou_mdp_change_ne_revient_pas() -> anyhow::Result<()> {
        let state = etat().await?;
        let user = utilisateur(&state).await?;
        let sid1 = ouvrir(&state, &user).await?;
        let sid2 = ouvrir(&state, &user).await?;
        assert_eq!(lignes(&state).await, 2);
        fermer_utilisateur(&state, 7).await;
        assert_eq!(lignes(&state).await, 0);
        assert!(state.sessions.is_empty());
        assert!(retrouver(&state, &sid1, 1_000).await.is_none());
        assert!(retrouver(&state, &sid2, 1_000).await.is_none());

        // Compte désactivé sans passer par la purge : la ligne n'est pas rechargée.
        let sid3 = ouvrir(&state, &user).await?;
        sqlx::query("UPDATE utilisateur SET actif=0 WHERE id=7").execute(&state.pool).await?;
        state.sessions.clear();
        assert!(retrouver(&state, &sid3, 1_000).await.is_none());
        assert_eq!(lignes(&state).await, 0);
        Ok(())
    }
}

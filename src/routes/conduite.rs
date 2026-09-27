//! Conduite en bandes : calendrier semaine par semaine, listes de travail,
//! places et flux, analyse comparée des bandes.
//!
//! Tout part d'un même chargement (`charger_bandes`) : pour chaque bande, ce
//! qui s'est réellement passé (mises-bas, sevrages, pertes, ventes) et, pour ce
//! qui n'a pas encore eu lieu, une prévision fondée sur l'historique de
//! l'élevage (taux de mise-bas, sevrés par portée). Les dates viennent des
//! réglages de conduite (`load_band_schedule`), comme le planning.

use super::*;
use chrono::Datelike;

/// Horizon des alertes d'étape (« entrée en maternité dans 10 jours »).
const ALERTE_ETAPE_JOURS: i64 = 10;
/// En deçà, l'alerte d'étape passe en « attention ».
const ALERTE_ETAPE_PROCHE: i64 = 3;
/// Horizon des prévisions d'occupation surveillées par les alertes.
const ALERTE_OCCUPATION_SEMAINES: i64 = 16;
/// Valeurs par défaut quand l'élevage n'a pas encore d'historique.
const TAUX_MISE_BAS_DEFAUT: f64 = 0.85;
const SEVRES_PORTEE_DEFAUT: f64 = 12.0;

// ─────────────────────────── Étapes ───────────────────────────

#[derive(Clone)]
struct Etape {
    code: String,
    libelle: String,
    court: String,
    jour: i64,
    /// Charge de travail par animal concerné (truie ou porc), en points.
    poids: f64,
    sur_truies: bool,
    /// Acte de protocole sanitaire (vaccin, traitement) représenté.
    acte_id: Option<i64>,
    /// L'étape tombe le lundi qui suit `jour` (IA des truies sevrées : du
    /// lundi au mercredi de la semaine après le sevrage).
    lundi_suivant: bool,
}

impl Etape {
    fn fixe(code: &str, libelle: &str, court: &str, jour: i64, poids: f64, sur_truies: bool) -> Self {
        Etape {
            code: code.into(),
            libelle: libelle.into(),
            court: court.into(),
            jour,
            poids,
            sur_truies,
            acte_id: None,
            lundi_suivant: false,
        }
    }

    fn date(&self, date_mb: NaiveDate) -> NaiveDate {
        let date = date_mb + Duration::days(self.jour);
        if self.lundi_suivant {
            lundi(date) + Duration::days(7)
        } else {
            date
        }
    }
}

fn etapes(c: BandSchedule) -> Vec<Etape> {
    let mut ia_retour = Etape::fixe(
        "ia_retour",
        "IA des truies sevrées (lundi → mercredi)",
        "IA sevrées",
        c.weaning,
        1.0,
        true,
    );
    ia_retour.lundi_suivant = true;
    vec![
        Etape::fixe("ia", "Inséminations", "IA", -c.gestation, 1.0, true),
        Etape::fixe("retour", "Contrôle des retours en chaleur", "Retours", -c.gestation + 21, 0.3, true),
        Etape::fixe("echo", "Échographies", "Écho", -c.gestation + c.echo_after_ia, 0.3, true),
        Etape::fixe("gestante", "Passage en gestante", "→ Gestante", -c.gestation + c.echo_after_ia + 2, 0.3, true),
        Etape::fixe("maternite", "Entrée en maternité", "Entrée mater.", -c.maternity_before_farrowing, 0.5, true),
        Etape::fixe("mise_bas", "Mises-bas", "Mises-bas", 0, 2.0, true),
        Etape::fixe("sevrage", "Sevrage", "Sevrage", c.weaning, 1.0, true),
        Etape::fixe("verraterie", "Passage en verraterie des truies sevrées", "→ Verraterie", c.weaning, 0.3, true),
        ia_retour,
        Etape::fixe("transfert", "Transfert en engraissement", "Transfert", c.transfer_finishing, 0.05, false),
        Etape::fixe("depart", "Départs abattoir", "Départ", c.departure, 0.05, false),
    ]
}

/// Étapes fixes, plus les vaccins et traitements des protocoles sanitaires
/// des truies, placés par rapport à l'IA (référence « gestation »), à la
/// mise-bas ou au sevrage.
async fn toutes_etapes(pool: &SqlitePool, c: BandSchedule) -> AppResult<Vec<Etape>> {
    let mut liste = etapes(c);
    let actes: Vec<(i64, String, String, i64, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT id, libelle, lower(reference), jour, produit, categorie FROM acteprotocole \
         WHERE actif=1 AND (cible LIKE '%femelle%' OR cible LIKE '%truie%')",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (id, libelle, reference, jour, produit, categorie) in actes {
        let base = match reference.as_str() {
            "gestation" | "ia" | "insemination" => -c.gestation,
            "mise_bas" | "misebas" | "mb" | "lactation" => 0,
            "sevrage" => c.weaning,
            _ => continue,
        };
        let vaccin = categorie.as_deref() == Some("vaccin");
        liste.push(Etape {
            code: "soin".into(),
            libelle: format!(
                "{} : {libelle}{}",
                if vaccin { "Vaccin" } else { "Traitement" },
                produit.map(|p| format!(" ({p})")).unwrap_or_default()
            ),
            court: format!("{} {libelle}", if vaccin { "💉" } else { "💊" }),
            jour: base + jour,
            poids: 0.4,
            sur_truies: true,
            acte_id: Some(id),
            lundi_suivant: false,
        });
    }
    liste.sort_by_key(|e| e.jour);
    Ok(liste)
}

/// Actes sanitaires réalisés : (acte, bande) → date de réalisation.
async fn actes_faits(pool: &SqlitePool) -> HashMap<(i64, i64), String> {
    sqlx::query_as::<_, (i64, i64, String)>(
        "SELECT acte_id, bande_id, MAX(date_realise) FROM acterealise GROUP BY acte_id, bande_id",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(acte, bande, date)| ((acte, bande), date))
    .collect()
}

/// « B2.12 » → « 2 » ; « 3.40 » → « 3 ». Sert à regrouper les cycles d'une
/// même bande (conduite en 3 bandes : B1, B2, B3).
fn groupe(code: &str) -> String {
    let sans_prefixe = code.trim().trim_start_matches(['B', 'b']);
    let groupe: String = sans_prefixe
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if groupe.is_empty() {
        code.to_string()
    } else {
        groupe
    }
}

fn lundi(date: NaiveDate) -> NaiveDate {
    date - Duration::days(i64::from(date.weekday().num_days_from_monday()))
}

fn jour_semaine(date: NaiveDate) -> &'static str {
    ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."]
        [date.weekday().num_days_from_monday() as usize]
}

fn quand(jours: i64, date: NaiveDate) -> String {
    match jours {
        0 => "aujourd’hui".into(),
        1 => "demain".into(),
        n => format!(
            "dans {n} j ({} {})",
            jour_semaine(date),
            date.format("%d/%m")
        ),
    }
}

/// Niveau de charge d'une semaine par rapport à la moyenne des semaines
/// travaillées de l'horizon affiché.
fn niveau_charge(charge: f64, moyenne: f64) -> &'static str {
    if charge <= 0.0 || moyenne <= 0.0 {
        return "calme";
    }
    let ratio = charge / moyenne;
    if ratio >= 1.8 {
        "pic"
    } else if ratio >= 1.3 {
        "chargee"
    } else if ratio < 0.6 {
        "calme"
    } else {
        "normale"
    }
}

// ─────────────────────────── Bandes ───────────────────────────

#[derive(Clone, Debug, Default)]
struct BandeCycle {
    id: i64,
    code: String,
    groupe: String,
    active: bool,
    site: Option<String>,
    date_mb: Option<NaiveDate>,
    truies: i64,
    ia_truies: i64,
    portees: i64,
    nt: i64,
    nv: i64,
    mn: i64,
    pertes: i64,
    sevrages: i64,
    sevres: i64,
    poids_sevrage: Option<f64>,
    morts_post: i64,
    vendus: i64,
}

/// Références de l'élevage tirées de l'historique, pour prévoir ce qui n'a
/// pas encore eu lieu.
#[derive(Clone, Copy, Debug)]
struct References {
    taux_mise_bas: f64,
    sevres_portee: f64,
}

impl BandeCycle {
    fn truies_prevues(&self, r: References) -> f64 {
        if self.portees > 0 {
            self.portees as f64
        } else {
            // Une bande en cours d'insémination n'a qu'une partie de ses IA
            // enregistrées : on part du plus grand entre truies inséminées et
            // truies rattachées à la bande.
            self.ia_truies.max(self.truies) as f64 * r.taux_mise_bas
        }
    }

    /// Porcs de la bande : réels après sevrage (sevrés − morts − vendus),
    /// porcelets sous la mère après mise-bas, sinon prévision.
    fn porcs(&self, r: References) -> f64 {
        if self.sevrages > 0 {
            (self.sevres - self.morts_post - self.vendus).max(0) as f64
        } else if self.portees > 0 {
            (self.nv - self.pertes).max(0) as f64
        } else {
            (self.truies_prevues(r) * r.sevres_portee).round()
        }
    }

    fn porcs_reels(&self) -> bool {
        self.sevrages > 0 || self.portees > 0
    }

    fn nv_portee(&self) -> Option<f64> {
        (self.portees > 0).then(|| self.nv as f64 / self.portees as f64)
    }

    fn sevres_portee(&self) -> Option<f64> {
        (self.sevrages > 0).then(|| self.sevres as f64 / self.sevrages as f64)
    }

    fn taux_pertes(&self) -> Option<f64> {
        (self.nv > 0 && self.sevrages > 0)
            .then(|| 100.0 * (self.nv - self.sevres).max(0) as f64 / self.nv as f64)
    }

    fn taux_mise_bas(&self) -> Option<f64> {
        (self.ia_truies > 0 && self.portees > 0)
            .then(|| 100.0 * self.portees as f64 / self.ia_truies as f64)
    }
}

const VENDUS_BANDE: &str = "SELECT CAST(COALESCE(SUM(CASE WHEN v.bande_id=?1 THEN COALESCE(v.nb_porcs,0) \
     WHEN v.bande_id IS NULL AND json_type(CASE WHEN json_valid(v.lots_json) THEN v.lots_json ELSE 'null' END)='array' \
     THEN (SELECT COALESCE(SUM(CAST(json_extract(j.value,'$.nb_porcs') AS INTEGER)),0) FROM json_each(v.lots_json) j \
     WHERE CAST(json_extract(j.value,'$.bande_id') AS INTEGER)=?1) ELSE 0 END),0) AS INTEGER) FROM venteapport v";

/// Morts après la maternité, sans recompter les porcelets déjà déclarés en
/// perte de maternité (la saisie rapide écrit les deux tables).
const MORTS_APRES_MATERNITE: &str = "SELECT CAST(COALESCE(SUM(d.nombre),0) AS INTEGER) FROM declarationmort d \
     WHERE d.bande_code=?1 AND (d.stade IS NULL OR d.stade NOT LIKE 'mater%') \
     AND NOT EXISTS(SELECT 1 FROM perteporcelet p JOIN bande b ON b.id=p.bande_id WHERE b.code=d.bande_code \
     AND p.date=d.date AND lower(COALESCE(p.cause,''))=lower(COALESCE(d.cause,'')))";

async fn charger_bandes(pool: &SqlitePool, archivees: i64) -> AppResult<Vec<BandeCycle>> {
    let lignes = sqlx::query(
        "SELECT id,code,site,date_mb,active FROM bande WHERE active=1 \
         OR id IN (SELECT id FROM bande WHERE active=0 AND date_mb IS NOT NULL AND trim(date_mb)<>'' ORDER BY date_mb DESC LIMIT ?) \
         ORDER BY date_mb IS NULL,date_mb",
    )
    .bind(archivees)
    .fetch_all(pool)
    .await?;
    let mut bandes = Vec::with_capacity(lignes.len());
    for ligne in lignes {
        let id: i64 = ligne.try_get("id")?;
        let code: String = ligne.try_get("code")?;
        let date_mb: Option<String> = ligne.try_get::<Option<String>, _>("date_mb").ok().flatten();
        let chiffres = sqlx::query(
            "SELECT \
             (SELECT COUNT(*) FROM truie WHERE reformee=0 AND bande_code=?2) AS truies,\
             (SELECT COUNT(DISTINCT truie_id) FROM evenement WHERE bande_id=?1 AND type='ia') AS ia_truies,\
             (SELECT COUNT(*) FROM evenement WHERE bande_id=?1 AND type='mise_bas' AND date<=date('now') AND COALESCE(nes_totaux,0)>0) AS portees,\
             (SELECT CAST(COALESCE(SUM(nes_totaux),0) AS INTEGER) FROM evenement WHERE bande_id=?1 AND type='mise_bas' AND date<=date('now')) AS nt,\
             (SELECT CAST(COALESCE(SUM(nes_vifs),0) AS INTEGER) FROM evenement WHERE bande_id=?1 AND type='mise_bas' AND date<=date('now')) AS nv,\
             (SELECT CAST(COALESCE(SUM(mort_nes),0) AS INTEGER) FROM evenement WHERE bande_id=?1 AND type='mise_bas' AND date<=date('now')) AS mn,\
             (SELECT CAST(COALESCE(SUM(nb),0) AS INTEGER) FROM perteporcelet WHERE bande_id=?1) AS pertes,\
             (SELECT COUNT(*) FROM evenement WHERE bande_id=?1 AND type='sevrage' AND date<=date('now')) AS sevrages,\
             (SELECT CAST(COALESCE(SUM(nb_sevres),0) AS INTEGER) FROM evenement WHERE bande_id=?1 AND type='sevrage' AND date<=date('now')) AS sevres,\
             (SELECT AVG(NULLIF(poids_moyen,0)) FROM evenement WHERE bande_id=?1 AND type='sevrage') AS poids",
        )
        .bind(id)
        .bind(&code)
        .fetch_one(pool)
        .await?;
        let morts_post: i64 = sqlx::query_scalar(MORTS_APRES_MATERNITE)
            .bind(&code)
            .fetch_one(pool)
            .await?;
        let vendus: i64 = sqlx::query_scalar(VENDUS_BANDE)
            .bind(id)
            .fetch_one(pool)
            .await?;
        bandes.push(BandeCycle {
            id,
            groupe: groupe(&code),
            code,
            active: ligne.try_get::<i64, _>("active").unwrap_or(0) == 1,
            site: ligne.try_get::<Option<String>, _>("site").ok().flatten(),
            date_mb: date_mb.as_deref().and_then(parse_stored_date),
            truies: chiffres.try_get("truies").unwrap_or(0),
            ia_truies: chiffres.try_get("ia_truies").unwrap_or(0),
            portees: chiffres.try_get("portees").unwrap_or(0),
            nt: chiffres.try_get("nt").unwrap_or(0),
            nv: chiffres.try_get("nv").unwrap_or(0),
            mn: chiffres.try_get("mn").unwrap_or(0),
            pertes: chiffres.try_get("pertes").unwrap_or(0),
            sevrages: chiffres.try_get("sevrages").unwrap_or(0),
            sevres: chiffres.try_get("sevres").unwrap_or(0),
            poids_sevrage: chiffres.try_get::<Option<f64>, _>("poids").ok().flatten(),
            morts_post,
            vendus,
        });
    }
    Ok(bandes)
}

fn references(bandes: &[BandeCycle]) -> References {
    let (portees, ia) = bandes
        .iter()
        .filter(|b| b.portees > 0 && b.ia_truies > 0)
        .fold((0, 0), |(p, i), b| (p + b.portees, i + b.ia_truies));
    let (sevres, sevrages) = bandes
        .iter()
        .filter(|b| b.sevrages > 0)
        .fold((0, 0), |(s, n), b| (s + b.sevres, n + b.sevrages));
    References {
        taux_mise_bas: if ia > 0 {
            (portees as f64 / ia as f64).clamp(0.5, 1.0)
        } else {
            TAUX_MISE_BAS_DEFAUT
        },
        sevres_portee: if sevrages > 0 {
            (sevres as f64 / sevrages as f64).clamp(6.0, 18.0)
        } else {
            SEVRES_PORTEE_DEFAUT
        },
    }
}

// ─────────────────────────── Capacités et occupation ───────────────────────────

#[derive(Clone, Copy, Debug, Default)]
struct Capacites {
    maternite: Option<i64>,
    post_sevrage: Option<i64>,
    engraissement: Option<i64>,
}

/// Places déclarées dans Paramètres. `parametre.places_*` (saisies place par
/// place) priment sur les anciens réglages `reglage.capacite_*`.
async fn capacites(pool: &SqlitePool) -> AppResult<Capacites> {
    async fn lire(pool: &SqlitePool, places: &str, ancien: &str) -> AppResult<Option<i64>> {
        let valeur: Option<i64> = sqlx::query_scalar(
            "SELECT CAST(valeur AS INTEGER) FROM parametre WHERE cle=? AND CAST(valeur AS INTEGER)>0",
        )
        .bind(places)
        .fetch_optional(pool)
        .await?;
        if valeur.is_some() {
            return Ok(valeur);
        }
        Ok(sqlx::query_scalar(
            "SELECT CAST(valeur AS INTEGER) FROM reglage WHERE cle=? AND CAST(valeur AS INTEGER)>0",
        )
        .bind(ancien)
        .fetch_optional(pool)
        .await?)
    }
    Ok(Capacites {
        maternite: lire(pool, "places_maternite", "capacite_maternite").await?,
        post_sevrage: lire(pool, "places_post_sevrage", "capacite_postsevrage").await?,
        engraissement: lire(pool, "places_engraissement", "capacite_engraissement").await?,
    })
}

/// Occupation prévue un jour donné : truies en maternité, porcs en
/// post-sevrage et en engraissement, avec les bandes concernées.
#[derive(Default)]
struct Occupation {
    mat: f64,
    ps: f64,
    engr: f64,
    b_mat: Vec<String>,
    b_ps: Vec<String>,
    b_engr: Vec<String>,
}

fn occupation(
    bandes: &[BandeCycle],
    c: BandSchedule,
    r: References,
    jour: NaiveDate,
    aujourd_hui: NaiveDate,
) -> Occupation {
    let mut o = Occupation::default();
    for bande in bandes {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        let age = (jour - date_mb).num_days();
        if age >= -c.maternity_before_farrowing && age < c.weaning {
            o.mat += bande.truies_prevues(r);
            o.b_mat.push(bande.code.clone());
        } else if age >= c.weaning && age < c.transfer_finishing {
            o.ps += bande.porcs(r);
            o.b_ps.push(bande.code.clone());
        } else if age >= c.transfer_finishing {
            let porcs = bande.porcs(r);
            // Au-delà du départ prévu, seuls comptent les porcs réellement
            // encore présents, et seulement dans les deux semaines à venir.
            let encore_la = age < c.departure
                || (bande.active
                    && bande.porcs_reels()
                    && porcs > 0.0
                    && (jour - aujourd_hui).num_days() <= 14);
            if encore_la && porcs > 0.0 {
                o.engr += porcs;
                o.b_engr.push(bande.code.clone());
            }
        }
    }
    o
}

fn etat_occupation(valeur: f64, capacite: Option<i64>) -> Value {
    let valeur = valeur.round();
    let taux = capacite
        .filter(|c| *c > 0)
        .map(|c| 100.0 * valeur / c as f64);
    let etat = match taux {
        Some(t) if t > 100.0 => "deborde",
        Some(t) if t >= 90.0 => "limite",
        Some(_) => "ok",
        None => "inconnu",
    };
    json!({"valeur": valeur.round() as i64, "capacite": capacite, "taux": taux.map(|t| t.round() as i64), "etat": etat})
}

// ─────────────────────────── Alertes ───────────────────────────

/// Alertes de conduite : étapes des 10 prochains jours et débordements
/// prévus. Utilisées par la cloche d'alertes et par le Rapport.
pub(super) async fn alertes(pool: &SqlitePool) -> AppResult<Vec<Value>> {
    let calendrier = load_band_schedule(pool).await?;
    let liste_etapes = toutes_etapes(pool, calendrier).await?;
    let bandes = charger_bandes(pool, 6).await?;
    let r = references(&bandes);
    let aujourd_hui = Local::now().date_naive();
    let mut sortie = Vec::new();
    for bande in bandes.iter().filter(|b| b.active) {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        for etape in &liste_etapes {
            let date = etape.date(date_mb);
            let jours = (date - aujourd_hui).num_days();
            if !(0..=ALERTE_ETAPE_JOURS).contains(&jours) {
                continue;
            }
            // Une étape déjà enregistrée n'est plus à annoncer.
            let deja_fait = match etape.code.as_str() {
                "mise_bas" => bande.portees > 0 && jours == 0,
                "sevrage" => bande.sevrages > 0,
                _ => false,
            };
            if deja_fait {
                continue;
            }
            sortie.push(json!({
                "message": format!("Bande {} : {} {}", bande.code, etape.libelle.to_lowercase(), quand(jours, date)),
                "url": format!("/conduite/semaine?debut={}", lundi(date)),
                "niveau": if jours <= ALERTE_ETAPE_PROCHE { "attention" } else { "information" },
                "date": date.to_string(),
                "jours": jours,
            }));
        }
    }
    let places = capacites(pool).await?;
    let mut deja = HashSet::new();
    for semaine in 0..ALERTE_OCCUPATION_SEMAINES {
        let debut = lundi(aujourd_hui) + Duration::weeks(semaine);
        let milieu = debut + Duration::days(2);
        let o = occupation(&bandes, calendrier, r, milieu, aujourd_hui);
        for (code, libelle, valeur, capacite, qui, unite) in [
            (
                "mat",
                "Maternité",
                o.mat,
                places.maternite,
                o.b_mat,
                "truies prévues",
            ),
            (
                "ps",
                "Post-sevrage",
                o.ps,
                places.post_sevrage,
                o.b_ps,
                "porcs prévus",
            ),
            (
                "engr",
                "Engraissement",
                o.engr,
                places.engraissement,
                o.b_engr,
                "porcs prévus",
            ),
        ] {
            let Some(capacite) = capacite else { continue };
            if valeur.round() <= capacite as f64 || !deja.insert(code) {
                continue;
            }
            let exces = (valeur - capacite as f64).round() as i64;
            sortie.push(json!({
                "message": format!(
                    "{libelle} : {} {unite} la semaine du {} pour {capacite} places (+{exces}) — bande(s) {}",
                    valeur.round() as i64,
                    debut.format("%d/%m"),
                    qui.join(", ")
                ),
                "url": "/flux",
                "niveau": if semaine < 4 { "urgent" } else { "attention" },
                "date": debut.to_string(),
                "jours": (debut - aujourd_hui).num_days(),
            }));
        }
    }
    sortie.sort_by(|a, b| {
        a["jours"]
            .as_i64()
            .cmp(&b["jours"].as_i64())
            .then_with(|| a["message"].as_str().cmp(&b["message"].as_str()))
    });
    Ok(sortie)
}

// ─────────────────────────── Calendrier ───────────────────────────

pub(super) async fn calendrier(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let nb_semaines: i64 = match query.get("semaines").map(String::as_str) {
        Some("12") => 12,
        Some("26") => 26,
        Some("52") => 52,
        _ => 18,
    };
    let calendrier = load_band_schedule(pool).await?;
    let liste_etapes = toutes_etapes(pool, calendrier).await?;
    let faits = actes_faits(pool).await;
    let bandes = charger_bandes(pool, 6).await?;
    let r = references(&bandes);
    let places = capacites(pool).await?;
    let aujourd_hui = Local::now().date_naive();
    let premier = lundi(aujourd_hui) - Duration::weeks(2);
    let dernier = premier + Duration::weeks(nb_semaines);

    // Tâches datées : elles comptent dans la charge de leur semaine.
    let taches: Vec<(String, String)> = sqlx::query_as(
        "SELECT echeance,titre FROM tache WHERE fait=0 AND echeance IS NOT NULL AND date(echeance) BETWEEN ? AND ?",
    )
    .bind(premier.to_string())
    .bind(dernier.to_string())
    .fetch_all(pool)
    .await?;

    let mut semaines: Vec<Map<String, Value>> = (0..nb_semaines)
        .map(|i| {
            let debut = premier + Duration::weeks(i);
            let mut m = Map::new();
            m.insert("debut".into(), json!(debut.to_string()));
            m.insert("numero".into(), json!(debut.iso_week().week()));
            m.insert("courante".into(), json!(debut == lundi(aujourd_hui)));
            m.insert("passee".into(), json!(debut < lundi(aujourd_hui)));
            m.insert("charge".into(), json!(0.0));
            m.insert("taches".into(), json!(0));
            m
        })
        .collect();

    let mut lignes = Vec::new();
    for bande in &bandes {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        // Une bande archivée n'apparaît que si une de ses étapes tombe dans
        // l'horizon affiché.
        let mut cellules: Vec<Vec<Value>> = vec![Vec::new(); nb_semaines as usize];
        let mut visible = false;
        for etape in &liste_etapes {
            let date = etape.date(date_mb);
            if date < premier || date >= dernier {
                continue;
            }
            visible = true;
            let index = ((date - premier).num_days() / 7) as usize;
            let animaux = if etape.sur_truies {
                bande.truies_prevues(r)
            } else {
                bande.porcs(r)
            };
            let charge = etape.poids * animaux;
            let courant = semaines[index]["charge"].as_f64().unwrap_or(0.0);
            semaines[index].insert("charge".into(), json!(courant + charge));
            cellules[index].push(json!({
                "code": &etape.code,
                "court": &etape.court,
                "libelle": &etape.libelle,
                "date": date.to_string(),
                "jour": jour_semaine(date),
                "animaux": animaux.round() as i64,
                "sur_truies": etape.sur_truies,
                "realise": realise(bande, etape, &faits),
            }));
        }
        if visible && (bande.active || cellules.iter().any(|c| !c.is_empty())) {
            lignes.push(json!({
                "id": bande.id,
                "code": bande.code,
                "groupe": bande.groupe,
                "active": bande.active,
                "cellules": cellules,
            }));
        }
    }
    for (echeance, _) in &taches {
        if let Some(date) = parse_stored_date(echeance) {
            let index = ((date - premier).num_days() / 7) as usize;
            if let Some(semaine) = semaines.get_mut(index) {
                let n = semaine["taches"].as_i64().unwrap_or(0) + 1;
                let charge = semaine["charge"].as_f64().unwrap_or(0.0) + 1.0;
                semaine.insert("taches".into(), json!(n));
                semaine.insert("charge".into(), json!(charge));
            }
        }
    }
    let charges: Vec<f64> = semaines
        .iter()
        .filter_map(|s| s["charge"].as_f64())
        .filter(|c| *c > 0.0)
        .collect();
    let moyenne = if charges.is_empty() {
        0.0
    } else {
        charges.iter().sum::<f64>() / charges.len() as f64
    };
    let maxi = charges.iter().copied().fold(0.0, f64::max);
    for semaine in &mut semaines {
        let charge = semaine["charge"].as_f64().unwrap_or(0.0);
        let debut = semaine["debut"]
            .as_str()
            .and_then(parse_stored_date)
            .unwrap_or(premier);
        let o = occupation(
            &bandes,
            calendrier,
            r,
            debut + Duration::days(2),
            aujourd_hui,
        );
        semaine.insert("niveau".into(), json!(niveau_charge(charge, moyenne)));
        semaine.insert(
            "hauteur".into(),
            json!(if maxi > 0.0 {
                (100.0 * charge / maxi).round()
            } else {
                0.0
            }),
        );
        semaine.insert("charge".into(), json!(charge.round() as i64));
        semaine.insert("mat".into(), etat_occupation(o.mat, places.maternite));
        semaine.insert("ps".into(), etat_occupation(o.ps, places.post_sevrage));
        semaine.insert("engr".into(), etat_occupation(o.engr, places.engraissement));
    }

    let mut ctx = context(&session);
    ctx.insert(
        "semaines".into(),
        Value::Array(semaines.into_iter().map(Value::Object).collect()),
    );
    ctx.insert("lignes".into(), Value::Array(lignes));
    ctx.insert("nb_semaines".into(), json!(nb_semaines));
    ctx.insert("alertes".into(), Value::Array(alertes(pool).await?));
    ctx.insert(
        "references".into(),
        json!({"taux_mise_bas": r.taux_mise_bas * 100.0, "sevres_portee": r.sevres_portee}),
    );
    render(&state, "conduite.html", Value::Object(ctx))
}

// ─────────────────────────── Liste de travail ───────────────────────────

/// Ce qui a déjà été enregistré pour une étape de la bande.
fn realise(bande: &BandeCycle, etape: &Etape, faits: &HashMap<(i64, i64), String>) -> Option<String> {
    if let Some(acte) = etape.acte_id {
        return faits.get(&(acte, bande.id)).map(|date| {
            let jour_mois = date.get(8..10).zip(date.get(5..7)).map(|(j, m)| format!("{j}/{m}"));
            format!("fait le {}", jour_mois.unwrap_or_else(|| date.clone()))
        });
    }
    match etape.code.as_str() {
        "ia" if bande.ia_truies > 0 => Some(format!("{} IA faite{}", bande.ia_truies, if bande.ia_truies > 1 { "s" } else { "" })),
        "mise_bas" if bande.portees > 0 => Some(format!("{} mise{}-bas", bande.portees, if bande.portees > 1 { "s" } else { "" })),
        "sevrage" if bande.sevrages > 0 => Some(format!("{} truie{} sevrée{}", bande.sevrages, if bande.sevrages > 1 { "s" } else { "" }, if bande.sevrages > 1 { "s" } else { "" })),
        _ => None,
    }
}

async fn truies_etape(pool: &SqlitePool, bande: &BandeCycle, code: &str, acte_id: i64) -> AppResult<Vec<Value>> {
    // Chaque requête prend les truies de la bande qui ont encore cette étape
    // à faire dans le cycle en cours.
    let condition = match code {
        "ia" => "NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='ia')",
        // IA après sevrage : sevrées avec cette bande, sans IA depuis.
        "ia_retour" => "EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='sevrage') \
                   AND NOT EXISTS(SELECT 1 FROM evenement e2 WHERE e2.truie_id=t.id AND e2.type='ia' \
                     AND e2.date >= (SELECT MAX(e3.date) FROM evenement e3 WHERE e3.truie_id=t.id AND e3.bande_id=?1 AND e3.type='sevrage'))",
        // Vaccins et traitements de gestation : truies non vides, pas encore mises bas.
        "soin" => "COALESCE((SELECT e.resultat FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type IN ('echo','echographie') ORDER BY e.date DESC,e.id DESC LIMIT 1),'pleine')<>'vide' \
                   AND NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='mise_bas' AND e.date<=date('now'))",
        // Retours en chaleur : inséminées, pas encore échographiées.
        "retour" => "EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='ia') \
                   AND NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type IN ('echo','echographie'))",
        // Gestante : écho pleine, encore logée en verraterie.
        "gestante" => "(SELECT e.resultat FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type IN ('echo','echographie') ORDER BY e.date DESC,e.id DESC LIMIT 1)='pleine' \
                   AND lower(COALESCE(s.type,s.nom,'')) LIKE '%verrat%'",
        // Verraterie : sevrée, encore logée en maternité.
        "verraterie" => "EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='sevrage') \
                   AND lower(COALESCE(s.type,s.nom,'')) LIKE '%mater%'",
        "echo" => "EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='ia') \
                   AND NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type IN ('echo','echographie'))",
        "maternite" | "mise_bas" => "COALESCE((SELECT e.resultat FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type IN ('echo','echographie') ORDER BY e.date DESC,e.id DESC LIMIT 1),'pleine')<>'vide' \
                   AND NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='mise_bas' AND e.date<=date('now') AND COALESCE(e.nes_totaux,0)>0)",
        "sevrage" => "EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='mise_bas') \
                   AND NOT EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='sevrage')",
        _ => return Ok(Vec::new()),
    };
    let appartenance = if matches!(code, "verraterie" | "ia_retour") {
        "t.bande_code=?2 OR EXISTS(SELECT 1 FROM evenement e WHERE e.truie_id=t.id AND e.bande_id=?1 AND e.type='sevrage')"
    } else {
        "t.bande_code=?2"
    };
    let sql = format!(
        "SELECT t.id,t.num_travail,t.rang,s.nom AS salle,c.nom AS case_nom,\
         (SELECT e.type||' le '||strftime('%d/%m',e.date) FROM evenement e WHERE e.truie_id=t.id ORDER BY e.date DESC,e.id DESC LIMIT 1) AS dernier \
         FROM truie t LEFT JOIN salle s ON s.id=t.salle_id LEFT JOIN casesalle c ON c.id=t.case_id \
         WHERE t.reformee=0 AND ({appartenance}) AND {condition} \
         AND NOT EXISTS(SELECT 1 FROM travail_fait f WHERE f.truie_id=t.id AND f.bande_id=?1 AND f.code=?3 AND f.acte_id=?4) \
         ORDER BY CAST(t.num_travail AS INTEGER),t.num_travail"
    );
    let lignes = sqlx::query(&sql)
        .bind(bande.id)
        .bind(&bande.code)
        .bind(code)
        .bind(acte_id)
        .fetch_all(pool)
        .await?;
    rows_to_json(lignes)
}

pub(super) async fn semaine(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let aujourd_hui = Local::now().date_naive();
    let debut = query
        .get("debut")
        .and_then(|d| parse_stored_date(d))
        .map(lundi)
        .unwrap_or_else(|| lundi(aujourd_hui));
    let fin = debut + Duration::days(6);
    let calendrier = load_band_schedule(pool).await?;
    let liste_etapes = toutes_etapes(pool, calendrier).await?;
    let faits = actes_faits(pool).await;
    let bandes = charger_bandes(pool, 6).await?;
    let r = references(&bandes);

    let mut travaux = Vec::new();
    for bande in &bandes {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        for etape in &liste_etapes {
            let date = etape.date(date_mb);
            if date < debut || date > fin {
                continue;
            }
            let acte_id = etape.acte_id.unwrap_or(0);
            let truies = if etape.sur_truies && bande.active {
                truies_etape(pool, bande, &etape.code, acte_id).await?
            } else {
                Vec::new()
            };
            // Truies déjà pointées pour cette étape (annulables).
            let faites = rows_to_json(
                sqlx::query(
                    "SELECT t.id,t.num_travail,f.date_fait,f.par FROM travail_fait f JOIN truie t ON t.id=f.truie_id \
                     WHERE f.code=? AND f.acte_id=? AND f.bande_id=? ORDER BY CAST(t.num_travail AS INTEGER),t.num_travail",
                )
                .bind(&etape.code)
                .bind(acte_id)
                .bind(bande.id)
                .fetch_all(pool)
                .await?,
            )?;
            travaux.push(json!({
                "date": date.to_string(),
                "jour": jour_semaine(date),
                "bande": bande.code,
                "bande_id": bande.id,
                "code": &etape.code,
                "libelle": &etape.libelle,
                "sur_truies": etape.sur_truies,
                "prevus": (if etape.sur_truies { bande.truies_prevues(r) } else { bande.porcs(r) }).round() as i64,
                "porcs_reels": bande.porcs_reels(),
                "realise": realise(bande, etape, &faits),
                "truies": truies,
                "faites": faites,
                "acte_id": acte_id,
            }));
        }
    }
    travaux.sort_by(|a, b| a["date"].as_str().cmp(&b["date"].as_str()));

    let bornes = [debut.to_string(), fin.to_string()];
    let soins = rows_to_json(
        sqlx::query(
            "SELECT sp.date_prevue,a.libelle,a.produit,a.dose,a.unite,COALESCE(b.code,'—') AS bande,COUNT(*) AS portees \
             FROM soinportee sp JOIN acteprotocole a ON a.id=sp.protocole_id LEFT JOIN bande b ON b.id=sp.bande_id \
             WHERE sp.date_realisee IS NULL AND date(sp.date_prevue) BETWEEN ? AND ? \
             GROUP BY sp.date_prevue,a.id,sp.bande_id ORDER BY sp.date_prevue",
        )
        .bind(&bornes[0])
        .bind(&bornes[1])
        .fetch_all(pool)
        .await?,
    )?;
    let rappels = rows_to_json(
        sqlx::query(
            "SELECT date(b.date_mb,printf('%+d day',ap.jour)) AS echeance,ap.libelle,ap.produit,b.code AS bande \
             FROM acteprotocole ap JOIN bande b ON b.active=1 \
             WHERE ap.actif=1 AND ap.rappel=1 AND ap.reference='mise_bas' AND b.date_mb IS NOT NULL \
             AND NOT EXISTS(SELECT 1 FROM acterealise ar WHERE ar.acte_id=ap.id AND ar.bande_id=b.id) \
             AND date(b.date_mb,printf('%+d day',ap.jour)) BETWEEN ? AND ? ORDER BY echeance",
        )
        .bind(&bornes[0])
        .bind(&bornes[1])
        .fetch_all(pool)
        .await?,
    )?;
    let taches = rows_to_json(
        sqlx::query(
            "SELECT echeance,titre,bande_code,salle FROM tache WHERE fait=0 AND echeance IS NOT NULL AND date(echeance) BETWEEN ? AND ? ORDER BY echeance",
        )
        .bind(&bornes[0])
        .bind(&bornes[1])
        .fetch_all(pool)
        .await?,
    )?;
    let entretiens = rows_to_json(
        sqlx::query(
            "SELECT nom,site,date(derniere_date,'+'||frequence_jours||' days') AS prochaine FROM entretien \
             WHERE derniere_date IS NOT NULL AND date(derniere_date,'+'||frequence_jours||' days') BETWEEN ? AND ? ORDER BY prochaine",
        )
        .bind(&bornes[0])
        .bind(&bornes[1])
        .fetch_all(pool)
        .await?,
    )?;

    let mut ctx = context(&session);
    ctx.insert("debut".into(), json!(debut.to_string()));
    ctx.insert("fin".into(), json!(fin.to_string()));
    ctx.insert("numero".into(), json!(debut.iso_week().week()));
    ctx.insert(
        "precedente".into(),
        json!((debut - Duration::weeks(1)).to_string()),
    );
    ctx.insert(
        "suivante".into(),
        json!((debut + Duration::weeks(1)).to_string()),
    );
    ctx.insert("travaux".into(), Value::Array(travaux));
    ctx.insert("soins".into(), Value::Array(soins));
    ctx.insert("rappels".into(), Value::Array(rappels));
    ctx.insert("taches".into(), Value::Array(taches));
    ctx.insert("entretiens".into(), Value::Array(entretiens));
    ctx.insert("aujourdhui".into(), json!(aujourd_hui.to_string()));
    ctx.insert("message".into(), json!(query.get("ok")));
    render(&state, "conduite_semaine.html", Value::Object(ctx))
}

/// Valide des truies cochées dans « Travail de la semaine » :
/// - `fait` : pointage (déplacement, retour…) ; pour un vaccin ou un
///   traitement, l'acte est aussi inscrit comme réalisé pour la bande ;
/// - `pleine` / `vide` : échographie enregistrée pour chaque truie ;
/// - `annuler` : retire le pointage.
pub(super) async fn valider(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let pool = &state.pool;
    let code = form_text(&form, "code").ok_or_else(|| AppError::Invalid("Étape manquante".into()))?;
    let bande_id = form_i64(&form, "bande_id").ok_or_else(|| AppError::Invalid("Bande manquante".into()))?;
    let acte_id = form_i64(&form, "acte_id").unwrap_or(0);
    let date = form_date_or_today(&form, "date")?;
    let action = form.get("action").map(String::as_str).unwrap_or("fait");
    let ids = form_selected_ids(&form, "truie_");
    if ids.is_empty() {
        return Err(AppError::Invalid("Cochez au moins une truie".into()));
    }
    let mut tx = pool.begin().await?;
    match action {
        "pleine" | "vide" => {
            for id in &ids {
                sqlx::query("INSERT INTO evenement(type,date,truie_id,bande_id,resultat,note,suivi_actif) VALUES('echo',?,?,?,?,'Travail de la semaine',0)")
                    .bind(&date)
                    .bind(id)
                    .bind(bande_id)
                    .bind(action)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        "annuler" => {
            for id in &ids {
                sqlx::query("DELETE FROM travail_fait WHERE code=? AND acte_id=? AND bande_id=? AND truie_id=?")
                    .bind(&code)
                    .bind(acte_id)
                    .bind(bande_id)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        _ => {
            for id in &ids {
                sqlx::query("INSERT OR IGNORE INTO travail_fait(code,acte_id,bande_id,truie_id,date_fait,par) VALUES(?,?,?,?,?,?)")
                    .bind(&code)
                    .bind(acte_id)
                    .bind(bande_id)
                    .bind(id)
                    .bind(&date)
                    .bind(&session.identifiant)
                    .execute(&mut *tx)
                    .await?;
            }
            if acte_id > 0 {
                let deja: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM acterealise WHERE acte_id=? AND bande_id=? AND date_realise=?")
                    .bind(acte_id)
                    .bind(bande_id)
                    .bind(&date)
                    .fetch_one(&mut *tx)
                    .await?;
                if deja == 0 {
                    sqlx::query("INSERT INTO acterealise(acte_id,bande_id,date_realise,note) VALUES(?,?,?,?)")
                        .bind(acte_id)
                        .bind(bande_id)
                        .bind(&date)
                        .bind(format!("Travail de la semaine ({})", session.identifiant))
                        .execute(&mut *tx)
                        .await?;
                }
            }
        }
    }
    tx.commit().await?;
    let debut = form_text(&form, "debut").unwrap_or_default();
    let n = ids.len();
    Ok(Redirect::to(&format!("/conduite/semaine?debut={debut}&ok={n}#etape-{code}-{bande_id}-{acte_id}")).into_response())
}

// ─────────────────────────── Places et flux ───────────────────────────

pub(super) async fn flux(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let calendrier = load_band_schedule(pool).await?;
    let bandes = charger_bandes(pool, 6).await?;
    let r = references(&bandes);
    let places = capacites(pool).await?;
    let aujourd_hui = Local::now().date_naive();

    // Occupation actuelle, salle par salle, d'après les inventaires et
    // transferts de cases.
    let salles = sqlx::query(
        "SELECT s.id,si.code AS site,s.nom,COALESCE(s.type,'') AS type,\
         (SELECT COUNT(*) FROM casesalle c WHERE c.salle_id=s.id) AS cases,\
         (SELECT CAST(COALESCE(SUM(nb_max_porcs),0) AS INTEGER) FROM casesalle c WHERE c.salle_id=s.id) AS places,\
         (SELECT COUNT(*) FROM truie t WHERE t.reformee=0 AND t.salle_id=s.id) AS truies \
         FROM salle s JOIN site si ON si.id=s.site_id ORDER BY si.code,s.ordre,s.nom",
    )
    .fetch_all(pool)
    .await?;
    let mut occupation_salles = Vec::new();
    for salle in salles {
        let id: i64 = salle.try_get("id")?;
        let cases: Vec<i64> = sqlx::query_scalar("SELECT id FROM casesalle WHERE salle_id=?")
            .bind(id)
            .fetch_all(pool)
            .await?;
        let mut porcs = 0;
        for case_id in cases {
            porcs += case_pig_count(pool, case_id).await?.max(0);
        }
        let type_salle: String = salle.try_get("type")?;
        let places_salle: i64 = salle.try_get("places")?;
        let truies: i64 = salle.try_get("truies")?;
        let cases_n: i64 = salle.try_get("cases")?;
        let (occupes, capacite) = if type_salle.contains("mater") {
            (truies, (cases_n > 0).then_some(cases_n))
        } else if type_salle.contains("verrat") || type_salle.contains("gest") {
            (truies, (places_salle > 0).then_some(places_salle))
        } else {
            (porcs, (places_salle > 0).then_some(places_salle))
        };
        occupation_salles.push(json!({
            "site": salle.try_get::<String, _>("site")?,
            "nom": salle.try_get::<String, _>("nom")?,
            "type": type_salle,
            "occupation": etat_occupation(occupes as f64, capacite),
            "truies": truies,
            "porcs": porcs,
        }));
    }

    // Prévision d'occupation par semaine.
    let mut prevision = Vec::new();
    for semaine in 0..20 {
        let debut = lundi(aujourd_hui) + Duration::weeks(semaine);
        let o = occupation(
            &bandes,
            calendrier,
            r,
            debut + Duration::days(2),
            aujourd_hui,
        );
        prevision.push(json!({
            "debut": debut.to_string(),
            "numero": debut.iso_week().week(),
            "mat": etat_occupation(o.mat, places.maternite), "b_mat": o.b_mat,
            "ps": etat_occupation(o.ps, places.post_sevrage), "b_ps": o.b_ps,
            "engr": etat_occupation(o.engr, places.engraissement), "b_engr": o.b_engr,
        }));
    }

    // Bandes en post-sevrage et engraissement : effectif et sortie prévue.
    let mut sorties = Vec::new();
    for bande in bandes.iter().filter(|b| b.active) {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        let age = (aujourd_hui - date_mb).num_days();
        if age < calendrier.weaning - 7 {
            continue;
        }
        let depart = date_mb + Duration::days(calendrier.departure);
        let transfert = date_mb + Duration::days(calendrier.transfer_finishing);
        let stade = if age < calendrier.weaning {
            "Sevrage imminent"
        } else if age < calendrier.transfer_finishing {
            "Post-sevrage"
        } else {
            "Engraissement"
        };
        let premiere_vente: Option<String> =
            sqlx::query_scalar("SELECT MIN(date) FROM venteapport WHERE bande_id=?")
                .bind(bande.id)
                .fetch_one(pool)
                .await?;
        sorties.push(json!({
            "id": bande.id,
            "code": bande.code,
            "site": bande.site,
            "stade": stade,
            "age": age,
            "sevres": bande.sevres,
            "morts": bande.morts_post,
            "vendus": bande.vendus,
            "porcs": bande.porcs(r).round() as i64,
            "reel": bande.sevrages > 0,
            "transfert": transfert.to_string(),
            "depart": depart.to_string(),
            "semaines_depart": (depart - aujourd_hui).num_days().div_euclid(7),
            "jours_depart": (depart - aujourd_hui).num_days(),
            "premiere_vente": premiere_vente,
        }));
    }

    // Prochains sevrages : places de post-sevrage libres au jour du sevrage.
    let mut sevrages = Vec::new();
    for bande in bandes.iter().filter(|b| b.active && b.sevrages == 0) {
        let Some(date_mb) = bande.date_mb else {
            continue;
        };
        let date = date_mb + Duration::days(calendrier.weaning);
        if date < aujourd_hui - Duration::days(7) {
            continue;
        }
        let o = occupation(&bandes, calendrier, r, date, aujourd_hui);
        sevrages.push(json!({
            "code": bande.code,
            "date": date.to_string(),
            "prevus": bande.porcs(r).round() as i64,
            "reel": bande.porcs_reels(),
            "occupation": etat_occupation(o.ps, places.post_sevrage),
            "bandes": o.b_ps,
        }));
    }

    let mut ctx = context(&session);
    ctx.insert("salles".into(), Value::Array(occupation_salles));
    ctx.insert("prevision".into(), Value::Array(prevision));
    ctx.insert("sorties".into(), Value::Array(sorties));
    ctx.insert("sevrages".into(), Value::Array(sevrages));
    ctx.insert(
        "capacites".into(),
        json!({"maternite": places.maternite, "post_sevrage": places.post_sevrage, "engraissement": places.engraissement}),
    );
    ctx.insert(
        "references".into(),
        json!({"taux_mise_bas": r.taux_mise_bas * 100.0, "sevres_portee": r.sevres_portee}),
    );
    ctx.insert(
        "alertes".into(),
        Value::Array(
            alertes(pool)
                .await?
                .into_iter()
                .filter(|a| a["url"] == "/flux")
                .collect(),
        ),
    );
    render(&state, "flux.html", Value::Object(ctx))
}

// ─────────────────────────── Analyse des bandes ───────────────────────────

fn moyenne(valeurs: &[f64]) -> Option<f64> {
    (!valeurs.is_empty()).then(|| valeurs.iter().sum::<f64>() / valeurs.len() as f64)
}

fn coefficient_variation(valeurs: &[f64]) -> Option<f64> {
    let m = moyenne(valeurs)?;
    if valeurs.len() < 2 || m <= 0.0 {
        return None;
    }
    let variance =
        valeurs.iter().map(|v| (v - m).powi(2)).sum::<f64>() / (valeurs.len() - 1) as f64;
    Some(100.0 * variance.sqrt() / m)
}

async fn homogeneite(pool: &SqlitePool, bande: &BandeCycle) -> AppResult<Value> {
    let dates: Vec<String> = sqlx::query_scalar(
        "SELECT date FROM evenement WHERE bande_id=? AND type='mise_bas' AND date<=date('now') AND COALESCE(nes_totaux,0)>0 ORDER BY date",
    )
    .bind(bande.id)
    .fetch_all(pool)
    .await?;
    let jours: Vec<NaiveDate> = dates.iter().filter_map(|d| parse_stored_date(d)).collect();
    let etalement = match (jours.first(), jours.last()) {
        (Some(a), Some(b)) => Some((*b - *a).num_days()),
        _ => None,
    };
    // Part des mises-bas groupées à ±2 jours de la date médiane.
    let groupees = jours.get(jours.len() / 2).map(|mediane| {
        let n = jours
            .iter()
            .filter(|d| (**d - *mediane).num_days().abs() <= 2)
            .count();
        100.0 * n as f64 / jours.len() as f64
    });
    let poids_sevrage: Vec<f64> = sqlx::query_scalar(
        "SELECT poids_moyen FROM evenement WHERE bande_id=? AND type='sevrage' AND COALESCE(poids_moyen,0)>0",
    )
    .bind(bande.id)
    .fetch_all(pool)
    .await?;
    let ventes = sqlx::query(
        "SELECT COUNT(*) AS apports,MIN(date) AS premier,MAX(date) AS dernier,MIN(poids_moyen) AS pmin,MAX(poids_moyen) AS pmax,\
         SUM(poids_total)/NULLIF(SUM(nb_porcs),0) AS pmoy,MIN(tmp) AS tmin,MAX(tmp) AS tmax FROM venteapport WHERE bande_id=?",
    )
    .bind(bande.id)
    .fetch_one(pool)
    .await?;
    let apports: i64 = ventes.try_get("apports").unwrap_or(0);
    let premier: Option<String> = ventes
        .try_get::<Option<String>, _>("premier")
        .ok()
        .flatten();
    let dernier: Option<String> = ventes
        .try_get::<Option<String>, _>("dernier")
        .ok()
        .flatten();
    let etalement_ventes = match (
        premier.as_deref().and_then(parse_stored_date),
        dernier.as_deref().and_then(parse_stored_date),
    ) {
        (Some(a), Some(b)) => Some((b - a).num_days()),
        _ => None,
    };
    Ok(json!({
        "etalement_mb": etalement,
        "mb_groupees": groupees.map(|v| v.round() as i64),
        "poids_sevrage_cv": coefficient_variation(&poids_sevrage),
        "poids_sevrage_min": poids_sevrage.iter().copied().reduce(f64::min),
        "poids_sevrage_max": poids_sevrage.iter().copied().reduce(f64::max),
        "apports": apports,
        "etalement_ventes": etalement_ventes,
        "poids_abattage_min": ventes.try_get::<Option<f64>, _>("pmin").ok().flatten(),
        "poids_abattage_max": ventes.try_get::<Option<f64>, _>("pmax").ok().flatten(),
        "poids_abattage": ventes.try_get::<Option<f64>, _>("pmoy").ok().flatten(),
        "tmp_min": ventes.try_get::<Option<f64>, _>("tmin").ok().flatten(),
        "tmp_max": ventes.try_get::<Option<f64>, _>("tmax").ok().flatten(),
    }))
}

/// Truies qui tirent la bande vers le bas, avec la raison.
async fn truies_faibles(pool: &SqlitePool, bande: &BandeCycle) -> AppResult<(Vec<Value>, f64)> {
    let lignes = sqlx::query(
        "SELECT t.id,t.num_travail,t.rang,e.nes_totaux AS nt,e.nes_vifs AS nv,e.mort_nes AS mn,\
         (SELECT CAST(COALESCE(SUM(p.nb),0) AS INTEGER) FROM perteporcelet p WHERE p.truie_id=t.id AND p.bande_id=?1) AS pertes,\
         (SELECT s.nb_sevres FROM evenement s WHERE s.truie_id=t.id AND s.bande_id=?1 AND s.type='sevrage' ORDER BY s.date DESC LIMIT 1) AS sevres,\
         (SELECT COUNT(*) FROM evenement i WHERE i.truie_id=t.id AND i.bande_id=?1 AND i.type='ia') AS ia,\
         (SELECT COUNT(*) FROM evenement x WHERE x.truie_id=t.id AND x.bande_id=?1 AND x.type IN ('echo','echographie') AND x.resultat='vide') AS vides,\
         (SELECT AVG(o.nes_vifs) FROM evenement o WHERE o.truie_id=t.id AND o.type='mise_bas' AND COALESCE(o.bande_id,0)<>?1 AND COALESCE(o.nes_totaux,0)>0 AND o.date<e.date) AS nv_avant,\
         (SELECT COUNT(*) FROM evenement o WHERE o.truie_id=t.id AND o.type='mise_bas' AND COALESCE(o.bande_id,0)<>?1 AND COALESCE(o.nes_totaux,0)>0 AND o.date<e.date) AS portees_avant \
         FROM evenement e JOIN truie t ON t.id=e.truie_id \
         WHERE e.bande_id=?1 AND e.type='mise_bas' AND e.date<=date('now') AND COALESCE(e.nes_totaux,0)>0",
    )
    .bind(bande.id)
    .fetch_all(pool)
    .await?;
    let nv_moy = bande.nv_portee().unwrap_or(0.0);
    let sev_moy = bande.sevres_portee();
    let mut sortie = Vec::new();
    let mut manque = 0.0;
    for l in lignes {
        let nv: i64 = l.try_get("nv").unwrap_or(0);
        let mn: i64 = l.try_get("mn").unwrap_or(0);
        let pertes: i64 = l.try_get("pertes").unwrap_or(0);
        let sevres: Option<i64> = l.try_get::<Option<i64>, _>("sevres").ok().flatten();
        let ia: i64 = l.try_get("ia").unwrap_or(0);
        let vides: i64 = l.try_get("vides").unwrap_or(0);
        let nv_avant: Option<f64> = l.try_get::<Option<f64>, _>("nv_avant").ok().flatten();
        let mut raisons = Vec::new();
        if (nv as f64) <= (nv_moy - 4.0).min(11.0) {
            raisons.push(format!("petite portée ({nv} nés vifs)"));
        }
        if mn >= 3 {
            raisons.push(format!("{mn} mort-nés"));
        }
        if nv > 0 && pertes >= 3 && pertes as f64 >= 0.25 * nv as f64 {
            raisons.push(format!("{pertes} porcelets perdus"));
        }
        if let (Some(s), Some(m)) = (sevres, sev_moy) {
            if (s as f64) <= m - 3.0 {
                raisons.push(format!("{s} sevrés"));
            }
        }
        if ia >= 2 || vides > 0 {
            raisons.push("retour / écho vide".to_string());
        }
        if raisons.is_empty() {
            continue;
        }
        let deja_faible = nv_avant.is_some_and(|avant| avant <= nv_moy - 2.0);
        if deja_faible {
            raisons.push("déjà faible aux portées précédentes".to_string());
        }
        manque += (nv_moy - nv as f64).max(0.0);
        sortie.push(json!({
            "id": l.try_get::<i64, _>("id")?,
            "numero": l.try_get::<String, _>("num_travail")?,
            "rang": l.try_get::<Option<i64>, _>("rang").ok().flatten(),
            "nt": l.try_get::<i64, _>("nt").unwrap_or(0),
            "nv": nv,
            "mn": mn,
            "pertes": pertes,
            "sevres": sevres,
            "nv_avant": nv_avant,
            "portees_avant": l.try_get::<i64, _>("portees_avant").unwrap_or(0),
            "raisons": raisons,
            "a_reformer": deja_faible,
        }));
    }
    sortie.sort_by(|a, b| {
        b["raisons"]
            .as_array()
            .map(Vec::len)
            .cmp(&a["raisons"].as_array().map(Vec::len))
            .then_with(|| a["nv"].as_i64().cmp(&b["nv"].as_i64()))
    });
    Ok((sortie, manque))
}

pub(super) async fn analyse(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Query(query): Query<HashMap<String, String>>,
) -> AppResult<Html<String>> {
    let pool = &state.pool;
    let bandes = charger_bandes(pool, 18).await?;
    let aujourd_hui = Local::now().date_naive();
    // Seules les bandes qui ont mis bas sont comparables.
    let faites: Vec<&BandeCycle> = bandes.iter().filter(|b| b.portees > 0).collect();

    let ligne = |b: &BandeCycle| {
        json!({
            "id": b.id, "code": b.code, "groupe": b.groupe, "active": b.active,
            "date_mb": b.date_mb.map(|d| d.to_string()),
            "ia": b.ia_truies, "portees": b.portees,
            "taux_mb": b.taux_mise_bas(),
            "nt_portee": (b.portees > 0).then(|| b.nt as f64 / b.portees as f64),
            "nv_portee": b.nv_portee(),
            "mn_portee": (b.portees > 0).then(|| b.mn as f64 / b.portees as f64),
            "sevres": b.sevres, "sevres_portee": b.sevres_portee(),
            "pertes": b.taux_pertes(), "poids_sevrage": b.poids_sevrage,
            "morts_post": b.morts_post, "vendus": b.vendus,
            "sevre": b.sevrages > 0,
        })
    };
    let lignes: Vec<Value> = faites.iter().rev().map(|b| ligne(b)).collect();

    // Moyennes de l'élevage et de chaque groupe (B1, B2, B3).
    let resume = |liste: &[&BandeCycle]| {
        let nv: Vec<f64> = liste.iter().filter_map(|b| b.nv_portee()).collect();
        let sev: Vec<f64> = liste.iter().filter_map(|b| b.sevres_portee()).collect();
        let pertes: Vec<f64> = liste.iter().filter_map(|b| b.taux_pertes()).collect();
        let tmb: Vec<f64> = liste.iter().filter_map(|b| b.taux_mise_bas()).collect();
        let sevres_bande: Vec<f64> = liste
            .iter()
            .filter(|b| b.sevrages > 0)
            .map(|b| b.sevres as f64)
            .collect();
        json!({
            "cycles": liste.len(),
            "nv_portee": moyenne(&nv), "sevres_portee": moyenne(&sev),
            "pertes": moyenne(&pertes), "taux_mb": moyenne(&tmb),
            "sevres_bande": moyenne(&sevres_bande),
        })
    };
    let elevage = resume(&faites);
    let mut codes_groupes: Vec<String> = faites.iter().map(|b| b.groupe.clone()).collect();
    codes_groupes.sort();
    codes_groupes.dedup();
    let groupes: Vec<Value> = codes_groupes
        .iter()
        .map(|g| {
            let liste: Vec<&BandeCycle> =
                faites.iter().copied().filter(|b| &b.groupe == g).collect();
            let mut r = resume(&liste);
            if let Some(obj) = r.as_object_mut() {
                obj.insert("groupe".into(), json!(g));
                obj.insert(
                    "derniers".into(),
                    json!(liste
                        .iter()
                        .rev()
                        .take(4)
                        .map(|b| ligne(b))
                        .collect::<Vec<_>>()),
                );
            }
            r
        })
        .collect();

    // Impact sur l'année : bandes sevrées depuis 12 mois.
    let il_y_a_un_an = aujourd_hui - Duration::days(365);
    let annee: Vec<&BandeCycle> = faites
        .iter()
        .copied()
        .filter(|b| b.sevrages > 0 && b.date_mb.is_some_and(|d| d >= il_y_a_un_an))
        .collect();
    let impact = if annee.len() >= 2 {
        let total: i64 = annee.iter().map(|b| b.sevres).sum();
        let moy = total as f64 / annee.len() as f64;
        let faible = annee.iter().min_by_key(|b| b.sevres).copied();
        let forte = annee.iter().max_by_key(|b| b.sevres).copied();
        faible.map(|f| {
            let gain = (moy - f.sevres as f64).max(0.0);
            json!({
                "bandes": annee.len(), "total": total, "moyenne": moy,
                "faible": f.code, "faible_sevres": f.sevres,
                "forte": forte.map(|b| b.code.clone()), "forte_sevres": forte.map(|b| b.sevres),
                "gain": gain.round() as i64,
                "gain_pct": if total > 0 { 100.0 * gain / total as f64 } else { 0.0 },
                "ecart_extremes": forte.map(|b| b.sevres - f.sevres),
            })
        })
    } else {
        None
    };

    // Bande détaillée : celle demandée, sinon la dernière qui a mis bas.
    let choisie = query
        .get("bande")
        .and_then(|code| faites.iter().copied().find(|b| &b.code == code))
        .or_else(|| faites.last().copied());
    let (detail, faibles, manque, homog) = match choisie {
        Some(b) => {
            let (faibles, manque) = truies_faibles(pool, b).await?;
            (
                Some(ligne(b)),
                faibles,
                manque,
                Some(homogeneite(pool, b).await?),
            )
        }
        None => (None, Vec::new(), 0.0, None),
    };
    let mut homogeneites = Vec::new();
    for b in faites.iter().rev().take(8) {
        let mut h = homogeneite(pool, b).await?;
        if let Some(obj) = h.as_object_mut() {
            obj.insert("code".into(), json!(b.code));
            obj.insert("id".into(), json!(b.id));
        }
        homogeneites.push(h);
    }

    let mut ctx = context(&session);
    ctx.insert("lignes".into(), Value::Array(lignes));
    ctx.insert("elevage".into(), elevage);
    ctx.insert("groupes".into(), Value::Array(groupes));
    ctx.insert("impact".into(), json!(impact));
    ctx.insert("detail".into(), json!(detail));
    ctx.insert("faibles".into(), Value::Array(faibles));
    ctx.insert("manque".into(), json!(manque.round() as i64));
    ctx.insert("homog".into(), json!(homog));
    ctx.insert("homogeneites".into(), Value::Array(homogeneites));
    ctx.insert(
        "choix".into(),
        json!(faites
            .iter()
            .rev()
            .map(|b| b.code.clone())
            .collect::<Vec<_>>()),
    );
    render(&state, "analyse_bandes.html", Value::Object(ctx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groupe_de_bande() {
        assert_eq!(groupe("B2.12"), "2");
        assert_eq!(groupe("b1.05"), "1");
        assert_eq!(groupe("3.40"), "3");
        assert_eq!(groupe("Cochettes"), "Cochettes");
    }

    #[test]
    fn niveaux_de_charge() {
        assert_eq!(niveau_charge(0.0, 10.0), "calme");
        assert_eq!(niveau_charge(5.0, 10.0), "calme");
        assert_eq!(niveau_charge(10.0, 10.0), "normale");
        assert_eq!(niveau_charge(14.0, 10.0), "chargee");
        assert_eq!(niveau_charge(20.0, 10.0), "pic");
    }

    #[test]
    fn prevision_des_porcs() {
        let r = References {
            taux_mise_bas: 0.8,
            sevres_portee: 12.0,
        };
        // Bande sevrée : effectif réel.
        let sevree = BandeCycle {
            sevrages: 25,
            sevres: 300,
            morts_post: 5,
            vendus: 100,
            ..Default::default()
        };
        assert_eq!(sevree.porcs(r), 195.0);
        // Bande qui n'a que des IA : 30 × 0,8 × 12.
        let a_venir = BandeCycle {
            ia_truies: 30,
            ..Default::default()
        };
        assert_eq!(a_venir.porcs(r), 288.0);
        // IA en cours : 2 enregistrées sur 17 truies rattachées.
        let en_cours = BandeCycle {
            ia_truies: 2,
            truies: 17,
            ..Default::default()
        };
        assert_eq!(en_cours.truies_prevues(r), 17.0 * 0.8);
        // Sous la mère : nés vifs moins pertes.
        let mater = BandeCycle {
            portees: 20,
            nv: 300,
            pertes: 40,
            ..Default::default()
        };
        assert_eq!(mater.porcs(r), 260.0);
    }

    #[test]
    fn lundi_de_la_semaine() {
        let jeudi = NaiveDate::from_ymd_opt(2026, 9, 24).unwrap();
        assert_eq!(lundi(jeudi), NaiveDate::from_ymd_opt(2026, 9, 21).unwrap());
    }

    #[test]
    fn debordement_detecte() {
        let c = BandSchedule::default();
        let r = References {
            taux_mise_bas: 0.85,
            sevres_portee: 12.0,
        };
        let jour = NaiveDate::from_ymd_opt(2026, 10, 1).unwrap();
        let bande = |code: &str, mb: NaiveDate| BandeCycle {
            code: code.into(),
            active: true,
            date_mb: Some(mb),
            sevrages: 26,
            sevres: 320,
            ..Default::default()
        };
        // Deux bandes en post-sevrage le même jour : 640 porcs.
        let bandes = vec![
            bande("B1", jour - Duration::days(c.weaning + 1)),
            bande("B2", jour - Duration::days(c.weaning + 20)),
        ];
        let o = occupation(&bandes, c, r, jour, jour);
        assert_eq!(o.ps, 640.0);
        assert_eq!(o.b_ps.len(), 2);
        assert_eq!(etat_occupation(o.ps, Some(310))["etat"], "deborde");
        assert_eq!(etat_occupation(280.0, Some(310))["etat"], "limite");
    }
}

#[cfg(test)]
mod essai_base_reelle {
    use super::*;

    /// Calendrier et listes de semaine sur une copie de la base réelle :
    /// `EO_TEST_BASE=… cargo test -- --ignored calendrier_sur_copie --nocapture`.
    #[tokio::test]
    #[ignore]
    async fn calendrier_sur_copie_de_la_base_reelle() -> anyhow::Result<()> {
        use super::super::documents::tests::session;
        let chemin = std::env::var("EO_TEST_BASE")?;
        let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&chemin).foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect_with(options).await?;
        crate::db::init(&pool).await?;
        let state = AppState::new(
            Config { bind: "127.0.0.1:8080".parse()?, db_path: chemin.into(), secure_cookies: false },
            pool,
            crate::templates::build()?,
        );
        let html = calendrier(State(state.clone()), Extension(session()), Query(HashMap::new())).await?.0;
        for code in ["ev-retour", "ev-gestante", "ev-verraterie", "IA faite", "ev-ia_retour", "Eryseng", "Suigen", "Pigfen"] {
            println!("calendrier : {code} présent = {}", html.contains(code));
        }
        let aujourd_hui = Local::now().date_naive();
        for semaine_n in 0..3 {
            let debut = lundi(aujourd_hui) + Duration::weeks(semaine_n);
            let mut q = HashMap::new();
            q.insert("debut".to_string(), debut.to_string());
            let page = semaine(State(state.clone()), Extension(session()), Query(q)).await?.0;
            let etapes: Vec<&str> = ["Contrôle des retours", "Passage en gestante", "Passage en verraterie", "IA des truies sevrées", "Vaccin", "Traitement", "Sevrage"]
                .into_iter()
                .filter(|e| page.contains(e))
                .collect();
            println!("semaine du {debut} : {etapes:?}");
        }
        Ok(())
    }
}

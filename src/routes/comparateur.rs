//! Comparateur de prix (/comparateur) : pour un produit à acheter, l'éleveur
//! note chaque offre (fournisseur, contenance d'une unité, nombre d'unités,
//! prix à l'unité ou pour le lot). La page ramène tout au même prix par
//! unité de mesure (€/kg, €/L…) et classe les offres de la moins chère à la
//! plus chère.

use super::*;

const UNITES: [&str; 6] = ["kg", "L", "dose", "pièce", "m", "t"];

/// Prix d'une unité achetée et prix par unité de mesure (si la contenance
/// est connue), à partir du prix total payé pour `nombre` unités.
pub(super) fn prix_ramenes(contenance: Option<f64>, nombre: f64, prix_total: f64) -> (f64, Option<f64>) {
    let par_unite = prix_total / nombre;
    (par_unite, contenance.filter(|c| *c > 0.0).map(|c| par_unite / c))
}

/// « 0,452 € » sous 10 €, « 12,40 € » au-dessus : les petits prix au kg ont
/// besoin d'un chiffre de plus pour se départager.
fn prix_lisible(x: f64) -> String {
    let s = if x.abs() < 10.0 { format!("{x:.3}") } else { format!("{x:.2}") };
    format!("{} €", s.replace('.', ","))
}

fn nombre_lisible(x: f64) -> String {
    let s = format!("{x:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.replace('.', ",")
}

/// Un achat lu dans les factures importées, ramené à une quantité dans
/// l'unité de base de sa famille (tonne d'aliment, flacon, dose, animal).
#[derive(Clone)]
pub(super) struct Achat {
    pub cle: String,
    pub famille: &'static str,
    pub date: String,
    pub produit: String,
    pub fournisseur: Option<String>,
    pub facture: Option<String>,
    pub quantite: f64,
    pub unite: &'static str,
    pub montant: f64,
}

fn espaces(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tous les achats des factures importées (aliment, véto, semence,
/// cochettes), du plus récent au plus ancien. Les lignes sans quantité ou
/// sans montant ne donnent pas de prix et sont laissées de côté.
pub(super) async fn achats_factures(pool: &SqlitePool) -> AppResult<Vec<Achat>> {
    type Ligne = (String, Option<String>, Option<String>, Option<String>, Option<String>, Option<f64>, Option<f64>);
    let familles: [(&'static str, &'static str, &'static str); 4] = [
        ("aliment", "t", "SELECT 'aliment:'||id,COALESCE(NULLIF(trim(date_reference),''),date),produit,fournisseur,num_facture,tonnage,montant_ht FROM livraisonaliment"),
        ("veto", "pièce", "SELECT 'veto:'||id,COALESCE(NULLIF(trim(date_reference),''),date),produit,fournisseur,num_facture,quantite,COALESCE(montant_ht,pu_ht*quantite) FROM achatveto"),
        ("semence", "dose", "SELECT 'semence:'||id,date,designation,fournisseur,num_facture,nb_doses,montant_ht FROM achatsemence"),
        ("genetique", "pièce", "SELECT 'genetique:'||id,date,designation,fournisseur,num_facture,nb_animaux,montant_ht FROM achatgenetique"),
    ];
    let mut achats = Vec::new();
    for (famille, unite, sql) in familles {
        let lignes: Vec<Ligne> = sqlx::query_as(sql).fetch_all(pool).await?;
        for (cle, date, produit, fournisseur, facture, quantite, montant) in lignes {
            let (Some(produit), Some(q), Some(m)) = (produit, quantite, montant) else { continue };
            let produit = espaces(&produit);
            if produit.is_empty() || q <= 0.0 || m <= 0.0 {
                continue;
            }
            achats.push(Achat {
                cle,
                famille,
                date: date.unwrap_or_default(),
                produit,
                fournisseur: fournisseur.map(|f| espaces(&f)).filter(|f| !f.is_empty()),
                facture,
                quantite: q,
                unite,
                montant: m,
            });
        }
    }
    achats.sort_by(|a, b| b.date.cmp(&a.date).then(a.produit.cmp(&b.produit)));
    Ok(achats)
}

fn famille_lisible(f: &str) -> &'static str {
    match f {
        "aliment" => "Aliment",
        "veto" => "Véto et hygiène",
        "semence" => "Semence",
        _ => "Animaux",
    }
}

/// Produits achetés, un par produit et fournisseur : dernier prix payé, prix
/// le plus bas et le plus haut, nombre d'achats.
pub(super) fn catalogue(achats: &[Achat]) -> Vec<Value> {
    let mut vus: Vec<(String, Value, f64, f64)> = Vec::new();
    for a in achats {
        let cle = format!("{}|{}|{}", a.famille, a.produit.to_uppercase(), a.fournisseur.as_deref().unwrap_or("").to_uppercase());
        let pu = a.montant / a.quantite;
        if let Some(v) = vus.iter_mut().find(|v| v.0 == cle) {
            v.2 = v.2.min(pu);
            v.3 = v.3.max(pu);
            let n = v.1["achats"].as_i64().unwrap_or(1) + 1;
            v.1["achats"] = json!(n);
            continue;
        }
        vus.push((
            cle,
            json!({"cle": a.cle, "famille": famille_lisible(a.famille), "produit": a.produit, "fournisseur": a.fournisseur,
                   "date": a.date, "unite": a.unite, "dernier": prix_lisible(pu), "achats": 1}),
            pu,
            pu,
        ));
    }
    vus.into_iter()
        .map(|(_, mut v, min, max)| {
            if (max - min).abs() > 0.005 {
                v["fourchette"] = json!(format!("{} à {}", prix_lisible(min), prix_lisible(max)));
            }
            v
        })
        .collect()
}

/// Facteur pour passer de l'unité de base d'un achat à l'unité du
/// comparatif (1 t = 1 000 kg). None : pas de conversion possible, on
/// compare alors au prix d'une unité achetée.
fn conversion(de: &str, vers: &str) -> Option<f64> {
    match (de, vers) {
        (a, b) if a == b => Some(1.0),
        ("t", "kg") => Some(1000.0),
        _ => None,
    }
}

async fn inserer_depuis_achat(pool: &SqlitePool, comparatif: i64, unite_comp: &str, a: &Achat) -> AppResult<()> {
    let etiquette = format!(
        "{} — facture{} du {}",
        a.produit,
        a.facture.as_deref().map(|n| format!(" {n}")).unwrap_or_default(),
        chrono::NaiveDate::parse_from_str(a.date.get(..10).unwrap_or(&a.date), "%Y-%m-%d")
            .map(|d| d.format("%d/%m/%Y").to_string())
            .unwrap_or_else(|_| a.date.clone())
    );
    sqlx::query(
        "INSERT INTO comparatif_offre(comparatif_id,produit,fournisseur,contenance,nombre,prix_total,prix_pour,note,source,date_achat) VALUES(?,?,?,?,?,?,'lot',?,?,?)",
    )
    .bind(comparatif)
    .bind(etiquette)
    .bind(&a.fournisseur)
    .bind(conversion(a.unite, unite_comp))
    .bind(a.quantite)
    .bind(a.montant)
    .bind(format!("Prix payé ({} {} achetés)", nombre_lisible(a.quantite), a.unite))
    .bind(&a.cle)
    .bind(&a.date)
    .execute(pool)
    .await?;
    Ok(())
}

async fn achat_par_cle(pool: &SqlitePool, cle: &str) -> AppResult<Achat> {
    achats_factures(pool)
        .await?
        .into_iter()
        .find(|a| a.cle == cle)
        .ok_or_else(|| AppError::Invalid("Achat introuvable dans les factures".into()))
}

/// « Comparer » depuis la liste des produits achetés : ouvre (ou crée) le
/// comparatif du produit et y met le dernier prix payé.
pub(super) async fn creer_depuis_facture(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let a = achat_par_cle(&state.pool, form.get("cle").map(String::as_str).unwrap_or_default()).await?;
    let unite = a.unite;
    let existant: Option<(i64, String)> = sqlx::query_as("SELECT id,unite FROM comparatif WHERE lower(trim(nom))=lower(trim(?)) ORDER BY id LIMIT 1")
        .bind(&a.produit)
        .fetch_optional(&state.pool)
        .await?;
    let (id, unite_comp) = match existant {
        Some(c) => c,
        None => {
            let id: i64 = sqlx::query_scalar("INSERT INTO comparatif(nom,unite,note) VALUES(?,?,?) RETURNING id")
                .bind(&a.produit)
                .bind(unite)
                .bind(format!("{} · d’après mes factures", famille_lisible(a.famille)))
                .fetch_one(&state.pool)
                .await?;
            (id, unite.to_string())
        }
    };
    let deja: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comparatif_offre WHERE comparatif_id=? AND source=?")
        .bind(id)
        .bind(&a.cle)
        .fetch_one(&state.pool)
        .await?;
    if deja == 0 {
        inserer_depuis_achat(&state.pool, id, &unite_comp, &a).await?;
    }
    db::journal(&state.pool, &session.nom, "comparer", "produit acheté", &a.produit, &format!("/comparateur/{id}")).await;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

/// Ajoute à un comparatif le prix payé sur une facture.
pub(super) async fn ajouter_depuis_facture(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let unite: String = sqlx::query_scalar("SELECT unite FROM comparatif WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?
        .ok_or(AppError::NotFound)?;
    let a = achat_par_cle(&state.pool, form.get("cle").map(String::as_str).unwrap_or_default()).await?;
    inserer_depuis_achat(&state.pool, id, &unite, &a).await?;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn liste(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
) -> AppResult<Html<String>> {
    let comparatifs = generic_rows(
        &state.pool,
        "SELECT c.id,c.nom,c.unite,c.note,substr(c.cree_le,1,10) AS cree_le,\
         (SELECT COUNT(*) FROM comparatif_offre o WHERE o.comparatif_id=c.id) AS offres \
         FROM comparatif c ORDER BY c.id DESC",
    )
    .await?;
    let mut ctx = context(&session);
    ctx.insert("comparatifs".into(), Value::Array(comparatifs));
    ctx.insert("produits".into(), Value::Array(catalogue(&achats_factures(&state.pool).await?)));
    ctx.insert("unites".into(), json!(UNITES));
    render(&state, "comparateur.html", Value::Object(ctx))
}

pub(super) async fn creer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Nom du produit à comparer obligatoire".into()))?;
    let unite = form_text(&form, "unite").unwrap_or_else(|| "kg".into());
    let id: i64 = sqlx::query_scalar("INSERT INTO comparatif(nom,unite,note) VALUES(?,?,?) RETURNING id")
        .bind(&nom)
        .bind(&unite)
        .bind(form_text(&form, "note"))
        .fetch_one(&state.pool)
        .await?;
    db::journal(&state.pool, &session.nom, "créer", "comparatif", &nom, &format!("/comparateur/{id}")).await;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

/// Offres d'un comparatif, calculées et classées. Toutes les offres ont une
/// contenance : classement au prix par unité de mesure ; sinon au prix d'une
/// unité achetée (on ne mélange pas les deux).
pub(super) async fn offres(pool: &SqlitePool, id: i64) -> AppResult<(Vec<Value>, bool)> {
    type Ligne = (i64, String, Option<String>, Option<f64>, f64, f64, String, Option<String>, Option<String>);
    let lignes: Vec<Ligne> = sqlx::query_as(
        "SELECT id,produit,fournisseur,contenance,nombre,prix_total,prix_pour,note,source FROM comparatif_offre WHERE comparatif_id=? ORDER BY id",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let par_mesure = !lignes.is_empty() && lignes.iter().all(|l| l.3.is_some_and(|c| c > 0.0));
    let mut calc: Vec<(f64, Value)> = lignes
        .into_iter()
        .map(|(oid, produit, fournisseur, contenance, nombre, total, pour, note, source)| {
            let (unite, mesure) = prix_ramenes(contenance, nombre, total);
            let cle = if par_mesure { mesure.unwrap_or(unite) } else { unite };
            (
                cle,
                json!({"id": oid, "produit": produit, "fournisseur": fournisseur,
                       "contenance": contenance.map(nombre_lisible), "nombre": nombre_lisible(nombre),
                       "lot": nombre != 1.0, "prix_pour": pour, "total": total,
                       "par_unite": prix_lisible(unite), "par_mesure": mesure.map(prix_lisible),
                       "quantite_totale": contenance.map(|c| nombre_lisible(c * nombre)),
                       "note": note, "facture": source.is_some()}),
            )
        })
        .collect();
    calc.sort_by(|a, b| a.0.total_cmp(&b.0));
    let meilleur = calc.first().map(|c| c.0);
    let resultat = calc
        .into_iter()
        .enumerate()
        .map(|(rang, (cle, mut v))| {
            if let (Some(m), Some(obj)) = (meilleur, v.as_object_mut()) {
                obj.insert("rang".into(), json!(rang + 1));
                obj.insert("meilleur".into(), json!(rang == 0));
                let ecart = if m > 0.0 { (cle / m - 1.0) * 100.0 } else { 0.0 };
                obj.insert("ecart".into(), json!((ecart * 10.0).round() / 10.0));
            }
            v
        })
        .collect();
    Ok((resultat, par_mesure))
}

pub(super) async fn fiche(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
) -> AppResult<Html<String>> {
    let comp: Option<(i64, String, String, Option<String>)> =
        sqlx::query_as("SELECT id,nom,unite,note FROM comparatif WHERE id=?")
            .bind(id)
            .fetch_optional(&state.pool)
            .await?;
    let (id, nom, unite, note) = comp.ok_or(AppError::NotFound)?;
    let (liste, par_mesure) = offres(&state.pool, id).await?;
    let mut ctx = context(&session);
    ctx.insert("comp".into(), json!({"id": id, "nom": nom, "unite": unite, "note": note}));
    ctx.insert("offres".into(), Value::Array(liste));
    ctx.insert("par_mesure".into(), json!(par_mesure));
    ctx.insert("unites".into(), json!(UNITES));
    // Achats des factures pour « Ajouter un prix de mes factures » : ceux
    // dont le nom ressemble au comparatif d'abord.
    let mots: Vec<String> = espaces(&nom).to_uppercase().split(' ').filter(|m| m.len() >= 3).map(String::from).collect();
    let (mut proches, mut autres): (Vec<Value>, Vec<Value>) = (Vec::new(), Vec::new());
    for a in achats_factures(&state.pool).await? {
        let v = json!({"cle": a.cle, "libelle": format!("{} · {} · {} {} pour {}", a.date.get(..10).unwrap_or(&a.date), a.produit,
            nombre_lisible(a.quantite), a.unite, prix_lisible(a.montant)), "famille": famille_lisible(a.famille)});
        let p = a.produit.to_uppercase();
        if mots.iter().any(|m| p.contains(m.as_str())) { proches.push(v) } else { autres.push(v) }
    }
    ctx.insert("achats_proches".into(), Value::Array(proches));
    ctx.insert("achats_autres".into(), Value::Array(autres));
    render(&state, "comparateur.html", Value::Object(ctx))
}

pub(super) async fn modifier(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let nom = form_text(&form, "nom").ok_or_else(|| AppError::Invalid("Nom obligatoire".into()))?;
    let r = sqlx::query("UPDATE comparatif SET nom=?,unite=?,note=? WHERE id=?")
        .bind(nom)
        .bind(form_text(&form, "unite").unwrap_or_else(|| "kg".into()))
        .bind(form_text(&form, "note"))
        .bind(id)
        .execute(&state.pool)
        .await?;
    if r.rows_affected() != 1 {
        return Err(AppError::NotFound);
    }
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn ajouter_offre(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let existe: Option<i64> = sqlx::query_scalar("SELECT id FROM comparatif WHERE id=?")
        .bind(id)
        .fetch_optional(&state.pool)
        .await?;
    existe.ok_or(AppError::NotFound)?;
    let produit = form_text(&form, "produit").ok_or_else(|| AppError::Invalid("Nom du produit obligatoire".into()))?;
    let contenance = form_f64(&form, "contenance");
    if contenance.is_some_and(|c| c <= 0.0) {
        return Err(AppError::Invalid("La quantité dans une unité doit être positive".into()));
    }
    let nombre = form_f64(&form, "nombre").unwrap_or(1.0);
    if nombre <= 0.0 {
        return Err(AppError::Invalid("Le nombre d'unités doit être positif".into()));
    }
    let prix = form_f64(&form, "prix")
        .filter(|p| *p >= 0.0)
        .ok_or_else(|| AppError::Invalid("Prix obligatoire".into()))?;
    let pour = if form.get("prix_pour").map(String::as_str) == Some("lot") { "lot" } else { "unite" };
    let total = if pour == "lot" { prix } else { prix * nombre };
    sqlx::query(
        "INSERT INTO comparatif_offre(comparatif_id,produit,fournisseur,contenance,nombre,prix_total,prix_pour,note) VALUES(?,?,?,?,?,?,?,?)",
    )
    .bind(id)
    .bind(&produit)
    .bind(form_text(&form, "fournisseur"))
    .bind(contenance)
    .bind(nombre)
    .bind(total)
    .bind(pour)
    .bind(form_text(&form, "note"))
    .execute(&state.pool)
    .await?;
    db::journal(&state.pool, &session.nom, "ajouter", "offre comparateur", &produit, &format!("/comparateur/{id}")).await;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn supprimer_offre(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path((id, offre)): Path<(i64, i64)>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    sqlx::query("DELETE FROM comparatif_offre WHERE id=? AND comparatif_id=?")
        .bind(offre)
        .bind(id)
        .execute(&state.pool)
        .await?;
    Ok(Redirect::to(&format!("/comparateur/{id}")).into_response())
}

pub(super) async fn supprimer(
    State(state): State<AppState>,
    Extension(session): Extension<SessionData>,
    Path(id): Path<i64>,
    Form(form): Form<HashMap<String, String>>,
) -> AppResult<Response> {
    require_writer(&session)?;
    verify_csrf(&session, &form)?;
    let mut tx = state.pool.begin().await?;
    sqlx::query("DELETE FROM comparatif_offre WHERE comparatif_id=?").bind(id).execute(&mut *tx).await?;
    sqlx::query("DELETE FROM comparatif WHERE id=?").bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    db::journal(&state.pool, &session.nom, "supprimer", "comparatif", &id.to_string(), "/comparateur").await;
    Ok(Redirect::to("/comparateur").into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prix_ramenes_a_l_unite_et_au_kg() {
        // Sac de 25 kg à 12,50 € : 12,50 €/sac et 0,50 €/kg.
        assert_eq!(prix_ramenes(Some(25.0), 1.0, 12.5), (12.5, Some(0.5)));
        // Lot de 40 sacs de 25 kg pour 400 € : 10 €/sac et 0,40 €/kg.
        assert_eq!(prix_ramenes(Some(25.0), 40.0, 400.0), (10.0, Some(0.4)));
        // Sans contenance : seulement le prix par unité.
        assert_eq!(prix_ramenes(None, 4.0, 10.0), (2.5, None));
        assert_eq!(prix_lisible(0.4), "0,400 €");
        assert_eq!(prix_lisible(12.5), "12,50 €");
        assert_eq!(nombre_lisible(25.0), "25");
        assert_eq!(nombre_lisible(0.75), "0,75");
    }

    #[tokio::test]
    async fn offres_classees_du_moins_cher_au_plus_cher() {
        let state = crate::routes::documents::tests::state().await;
        let p = &state.pool;
        sqlx::raw_sql(
            "INSERT INTO comparatif(id,nom,unite) VALUES(1,'Chaux','kg');\
             INSERT INTO comparatif_offre(comparatif_id,produit,fournisseur,contenance,nombre,prix_total,prix_pour) VALUES\
             (1,'Sac 25 kg','A',25,1,12.5,'unite'),\
             (1,'Palette 40 x 25 kg','A',25,40,400,'lot'),\
             (1,'Big bag 500 kg','B',500,1,225,'unite');",
        )
        .execute(p)
        .await
        .unwrap();
        let (liste, par_mesure) = offres(p, 1).await.unwrap();
        assert!(par_mesure);
        let produits: Vec<&str> = liste.iter().map(|o| o["produit"].as_str().unwrap()).collect();
        assert_eq!(produits, ["Palette 40 x 25 kg", "Big bag 500 kg", "Sac 25 kg"]);
        assert_eq!(liste[0]["meilleur"], json!(true));
        assert_eq!(liste[0]["par_mesure"], json!("0,400 €"));
        assert_eq!(liste[1]["ecart"], json!(12.5));
        assert_eq!(liste[2]["ecart"], json!(25.0));
        // Une offre sans contenance : on compare au prix par unité achetée.
        sqlx::query("INSERT INTO comparatif_offre(comparatif_id,produit,nombre,prix_total,prix_pour) VALUES(1,'Seau',1,5,'unite')")
            .execute(p)
            .await
            .unwrap();
        let (liste, par_mesure) = offres(p, 1).await.unwrap();
        assert!(!par_mesure);
        assert_eq!(liste[0]["produit"], json!("Seau"));
    }

    #[tokio::test]
    async fn produits_des_factures_et_conversion_tonne_kilo() {
        let state = crate::routes::documents::tests::state().await;
        let p = &state.pool;
        sqlx::raw_sql(
            "INSERT INTO livraisonaliment(id,date,produit,fournisseur,tonnage,montant_ht) VALUES\
             (1,'2026-10-01','GESTA PLUS','Coop',10,2900),(2,'2026-11-01','GESTA  PLUS','Coop',5,1500),(3,'2026-11-02','Vide','Coop',0,0);\
             INSERT INTO achatveto(id,date,produit,fournisseur,quantite,pu_ht,montant_ht) VALUES(1,'2026-10-26','VIRBAGEST FL 900ML','Coop',1,94.36,94.36);",
        )
        .execute(p)
        .await
        .unwrap();
        let achats = achats_factures(p).await.unwrap();
        assert_eq!(achats.len(), 3, "la ligne sans quantité ni montant est ignorée");
        let cat = catalogue(&achats);
        let gesta = cat.iter().find(|c| c["produit"] == json!("GESTA PLUS")).unwrap();
        assert_eq!(gesta["achats"], json!(2));
        assert_eq!(gesta["dernier"], json!("300,00 €"));
        assert_eq!(gesta["fourchette"], json!("290,00 € à 300,00 €"));
        // Un achat d'aliment (en tonnes) dans un comparatif au kg.
        sqlx::query("INSERT INTO comparatif(id,nom,unite) VALUES(1,'Gesta','kg')").execute(p).await.unwrap();
        let a = achats.iter().find(|a| a.cle == "aliment:2").unwrap();
        inserer_depuis_achat(p, 1, "kg", a).await.unwrap();
        let (liste, par_mesure) = offres(p, 1).await.unwrap();
        assert!(par_mesure);
        assert_eq!(liste[0]["par_unite"], json!("300,00 €"));
        assert_eq!(liste[0]["par_mesure"], json!("0,300 €"));
        assert_eq!(liste[0]["facture"], json!(true));
    }
}

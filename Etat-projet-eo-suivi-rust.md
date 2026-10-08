# EO-Suivi Élevage Rust — État du projet

Version actuelle : **2.60.0** — plan de vaccination en frise.

### Version 2.48.2 — Accueil sans bandeau d'alertes en double

Version Rust 2.60.0 : /sanitaire/plan (src/routes/vaccination.rs, sanitaire_plan.html) — frise éditable des actes acteprotocole par ligne cochettes (réf. arrivee) / truies (J0 = IA, réf. existantes conservées) / porcelets (réf. mise_bas → soinportee) / verrats (réf. annee) ; POST JSON dans un champ de formulaire, actes retirés → actif=0.

Version Rust 2.59.0 : /maternite vue mises-bas — modèle A choisi par Emmanuel : tableau (tr.mb-ligne#truie-ID) + panneau (div.mb-fiche#fiche-ID) ; mêmes formulaires et routes qu'avant (misebas, etat, perte), champs secondaires sous « Plus ».

Version Rust 2.58.8 : accueil_cockpit « À faire » — case = formulaire POST /taches/{id}/fait (retour=accueil → redirection /), auparavant case disabled décorative.

Version Rust 2.58.7 : economic_import::extract_pdf_text — si lopdf échoue au chargement ou ne trouve aucune page (PDF Cooperl LDPRX 4.54, apport 226101280151), lecture par `pdftotext -layout` (pdftotext_secours, dossier temporaire supprimé).

Version Rust 2.58.6 : formulaire disjoncteur liste les io 65552..65563 de chaque hôte (entrees_ipx) ; code n:hote|io crée le ipx_point (role disjoncteur, nom_entree) dans enregistrer.

Version Rust 2.58.5 : automatique_simple.html (questions numérotées, remplit c0_*/a0_* cachés ; champ mode=essai|vrai lu par enregistrer) pour modele=minuterie|disjoncteur ; libellés de automatique_regle.html réécrits (Quand… / Alors faire…).

Version Rust 2.58.4 : modèle « minuterie » dans MODELES (condition point = 1 pendant 30 min → action début valeur 0 sur la même sortie).

Version Rust 2.58.3 : macro onglets (ambiance_automate_onglets.html) liste les TYPES_V5 + Accès, titre Ipx800v5 ; GET /ambiance/automate, /ambiance/ipx, /ambiance/ipx/carte, /ambiance/ipx/explorateur redirigent vers /ambiance/ipx/v5/relais.

Version Rust 2.58.2 : onglets de l'automate sans Éclairage ni Automatique ; TYPES_V5 en blocs (BlocV5 plage ou motif) avec extensions X-THL (ana X-THL;XTHL + io ]Fault), X-010V (ana + io x-010v), X-PSU (io [XPSU]) ; macro automatique_onglets.html (Règles, Éclairage, Nouvelle règle) utilisée par automatique*.html et ambiance_eclairage*.html.

Version Rust 2.58.1 : /automatique/nouvelle propose des modèles (MODELES : lumière → /ambiance/eclairage/nouveau, alerte température, brumisation, chauffage, hygrométrie, porte la nuit, règle vide) ; /automatique liste aussi les programmes d'éclairage (eclairage_programme/eclairage_etat) ; une règle ne peut pas commander une lampe d'un programme d'éclairage actif (conflit avec eo-regul).

Version Rust 2.58.0 : module Automatique (routes/automatique.rs, migration 0028, /automatique) — règles si… alors… : conditions sur ipx_point (IPX et Zigbee via ambiance::lire_point), température/hygrométrie de salle (ambiance_etat, périmée après 15 min), phase soupe ; logique et/ou, délai, plage horaire ; actions commande (pilotable seulement, refus si ventilation en régulation, journal ipx_commande_journal utilisateur='automatique') et SMS, au début ou à la fin ; boucle 10 s lancée par main ; règles neuves en essai (journal seul) ; capteur muet = ni déclenchement ni fin. Test les_pages_se_rendent rend automatique*.html et ambiance_carte.html. 2.57.0 corrigée : filtre urlencode inconnu sur les pages IPX800 V5. /decouvrir redirige vers le site vitrine https://eo-elevage.fr/eo-sevel/.

Version Rust 2.57.0 : onglet « IPX800 V5 » de l'automate (/ambiance/ipx/v5/{relais,digitales,analogiques,opto,collecteurs}) — une page par type de la carte mère, plages d'identifiants fixes (TYPES_V5 dans routes/ambiance.rs), sans réglage de section ; ipx_carte et ipx_v5 partagent ambiance::carte ; champ « retour » des formulaires affecter/commander pour revenir sur la page V5.

Version Rust 2.56.1 : frise des bandes — nom du stade écrit dans chaque segment, segment en cours entouré (en_cours) ; /planning reprend la frise (accueil::frise_bandes) + tableau « Les bandes en détail » (stade, animaux, salles, prochaine étape, dates de chaque période).

Version Rust 2.56.0 : /eau (même page que /electricite, Genre::Eau) — conso d'eau par jour = écart entre relevés réparti par jour, vues semaine/mois/année, prix parametre eau_prix_m3 ; sous-compteurs électriques parametre elec_parent_<ieee> (barres empilées, « dont … »).

Version Rust 2.55.1 : table elec_heure (migration 0027) — historique horaire importé de Home Assistant (statistiques sensor.tri_energy / sensor.machine_a_soupe_energy, 08/03→25/09/2026, rattrapages après silence répartis, estime=1) ; vue Jour sans mesures → barres par heure importées.

Version Rust 2.55.0 : /electricite (src/routes/electricite.rs, section energie) — consommation par compteur Zigbee : jour (courbe ¼ h + kWh/heure), semaine, mois, année ; kWh = intégrale de total_power|power (trous > 10 min ignorés), validé contre les relevés d'index ; table elec_jour (migration 0026, sans purge) complétée toutes les 6 h et à l'ouverture ; prix parametre elec_prix_kwh.

Version Rust 2.54.2 : /energie — tableau des relevés de chaque compteur replié (details.en-releves, résumé : nombre, dernier relevé, conso).

Version Rust 2.54.1 : /rapport section « Machine à soupe » (rapport::soupe_incidents) : cycles Test circuit→Test circuit avec phase 37 ≥ 3 min, reçu par vanne = somme des passages (max par suite de relevés), consignes de la séquence/formule sur 7 j, manques > 2 kg et > 15 % ; alarmes courtes seulement comptées. Phase 37 renommée en base (48 événements).

Version Rust 2.54.0 : soupe phase 37 « Alarme débit distribution » → alerte critique soupe_distribution (SMS) après 180 s.

Version Rust 2.53.1 : carte Assistant Claude dans /equipements, réglages lisibles en thème sombre.

Version Rust 2.53.0 : connecteur Claude MCP (/mcp/{jeton}, src/routes/mcp.rs), page /assistant (src/assistant.rs, SQL lecture seule), ancienne dictée vocale supprimée.

Version Rust 2.52.5 : Proxmox — adresse http forcée en https (la redirection perdait le jeton) ; refus 403 sur /nodes/{n}/status affiché au lieu d'une page vide.

Version Rust 2.52.4 : « QR code » au lieu de « code QR » (page Découvrir).

Version Rust 2.52.3 : suppression de salle — bloquée seulement par cases, truies ou transferts ; contrôles/alertes/observations/IPX/caméras détachés (salle_id NULL, nom conservé), réglages et mesures d'ambiance supprimés.

Version Rust 2.52.2 : synchronise_sortie_abattoir — une case avec inventaire (daté avant l'apport) prend l'effectif compté (case_pig_count) au lieu de l'estimation par transferts. Données prod 05/10 : inventaire Case 1 La Berru (id 1) dernier_transfert_id=146, apport 12 = 109 porcs de la case 47.

Version Rust 2.52.1 : téléphone — entrée « Salles » dans la barre du bas, groupe « Salles et ambiance » (.menu-tel, ignoré par le fil d'Ariane) dans le menu du compte : les menus du haut sont masqués sous 850 px.

Version Rust 2.52.0 : module Serveur Proxmox (/equipements/proxmox, src/proxmox.rs) : lecture seule par jeton PVEAuditor (parametre proxmox_url/jeton/secret), nœuds, VM/CT, stockages, sauvegardes vzdump ; boucle 5 min, SMS via Free SMS (proxmox_alertes, proxmox_suivis, proxmox_signales).

Version Rust 2.51.3 : /decouvrir alignée sur les rôles réels (salarié en consultation, conseillers via sections, prestataire) ; /contact public sans la note interne « Avant une ouverture publique ».

Version Rust 2.51.2 : /decouvrir enrichie (accueil, salles, automate, éclairage, soupe, GTE, résultats, vente directe, rôles, modules à la demande) ; bandeau vert plus fixe (section au lieu de header, sticky global).

Version Rust 2.51.1 : pied de page sans « Rust » ; page publique /decouvrir (présentation détaillée, maquettes qui se déplient au défilement), lien depuis la page de connexion.

Version Rust 2.51.0 : saisie rapide « Quai abattoir » — porcs sortis des salles mais encore à l'élevage jusqu'au bon d'apport (quai_porc/quai_affectation, repris en priorité par synchronise_sortie_abattoir) ; affichage /departs et Mes salles.

Version Rust 2.48.2 : accueils « cockpit » et « flux » : le bandeau rouge des alertes (accueil_alertes.html, supprimé) faisait doublon avec le bouton « ! » du menu (État de l’élevage) ; retiré. Bouton « Gérer les bandes » lisible : la règle `.rust-main a:not(…)` imposait sa couleur de lien au bouton plein (texte vert sur fond vert).

### Version 2.48.1 — Machine à soupe : sections repliables

Version Rust 2.48.1 : /equipements/soupe : « Repas des 7 derniers jours » et « Journal » deviennent des blocs `<details>` repliés par défaut ; la mise à jour en place (toutes les 10 s) conserve l'état ouvert/replié choisi.

### Version 2.48.0 — Frise des bandes agrandie et dans Places et flux

Version Rust 2.48.0 : accueil « cockpit » : la carte « Conduite des bandes » prend toute la largeur avec des lignes plus hautes (étiquettes 190 px, lignes 54 px) ; « Prochaines interventions » passe dans la rangée suivante. Places et flux (/flux) : même frise en tête de page (`accueil::frise_bandes`, mêmes vues de bande et segments que l'accueil, gabarit `accueil_frise.html`). Le libellé « AUJOURD’HUI » ne chevauche plus le nom du mois.

### Versions 2.41.1 à 2.47.5 — résumé

Version Rust 2.47.5 : ce fichier n'avait plus été mis à jour depuis 2.41.0. Le détail de chaque version de 2.41.1 à 2.47.5 (machine à soupe : pilotage et synoptique ; import de factures ; truies improductives ; pages d'accueil au choix ; plans d'alimentation ; redevance DanBred) est dans `git log` ; `templates/correctifs.html`, lui aussi arrêté à 2.39.3, a reçu une entrée récapitulative 2.47.5. `.github/workflows/rust.yml` (resté à 2.39.3) est resynchronisé. Les tests remis en état à cette occasion (schéma de test sans `inventairecase.dernier_transfert_id` ni `zigbee_demarrage_reglage`, assertions périmées) ne changent pas le comportement de l'application.

### Version 2.41.0 — Ventilation des salles vides

Version Rust 2.41.0 : réglage par salle « Ventilation salle vide (%) » (fiche salle → Ventilation et consignes, colonne `ambiance_salle.ventil_vide`, vide = désactivé). eo-regul : si réglé, qu'aucune truie ni aucun porc n'est compté dans la salle (comptage case par case comme l'application) et qu'aucune arrivée n'est préparée (préconditionnement), la ventilation est tenue à ce pourcentage (mode `vide`, `regul_core::salle_vide`), sans alerte de température/hygrométrie. Badge « salle vide · ventilation réduite » sur /ambiance, « (salle vide) » sur la fiche.

### Version 2.40.1 — Machine à soupe : données de synchronisation

Version Rust 2.40.1 : les réponses de synchronisation du PC (R fiches vannes, S plans, U horaires, V système, W relais, X produits, Y formules…) sont gardées dans `soupe_donnee` (dernière réponse par commande) ; la page affiche la date de dernière synchronisation, les noms de produits (X, 16 premiers champs) et le nom du produit en dosage. Bloc « En direct » en grille. Constat : Mega-feeder n'interroge l'état (`c0`) que quand sa fenêtre de supervision est ouverte.

### Version 2.40.0 — Machine à soupe (écoute seule) et menu Caméras

Version Rust 2.40.0 : module facultatif « Machine à soupe » (Paramètres → Modules, `module_machine_soupe`, désactivé par défaut). Mes équipements → Machine à soupe (/equipements/soupe) : phase en direct, poids cuve, séquence/formule, vanne/consigne/contrôle/débit, repas des 7 derniers jours (prévu = consigne par vanne, distribué = différence du cumul machine), détail vanne par vanne, journal (phases + actions faites sur le PC). Alertes SMS via ambiance_alerte : `soupe_alerte` (phase 7 « alerte dosage ») et `soupe_silence` (> 10 min sans trame). Réception : POST /api/soupe/trames (jeton `soupe_jeton`, chemin public), service d'écoute Python servi par /api/soupe/script (scripts/soupe-ecoute.py, usbmon sur l'hôte Proxmox, lecture seule, contrôle vérifié). Migration 0024. Décodage : src/soupe.rs. Menu Mes équipements : ajout des entrées Caméras et Machine à soupe.

### Version 2.39.3 — Fusion carte de bande (2.38.1)

Version Rust 2.39.3 : fusion de la branche cameras (2.38.1, carte de bande) avec 2.39.2, après un déploiement de 2.38.1 qui avait retiré 2.38–2.39.2.

### Version 2.39.2 — Production par bande

Version Rust 2.39.2 : tableau pleine largeur ; BandeCycle.morts_reelles (sans Régul inventaire ni Départ / terminé) pour le taux de pertes de référence.

### Version 2.39.1 — Effectif après inventaire

Version Rust 2.39.1 : inventairecase.dernier_transfert_id (MAX(transfert.id) à la saisie) ; case_pig_count_conn et les deux CTE comptent les transferts du même jour d'id supérieur ; transfert case→même case neutre. Les anciens inventaires (NULL) gardent l'ancienne règle.

### Version 2.39.0 — Impression et cases incohérentes

Version Rust 2.39.0 : impression de /inseminations (CSS print) ; cases_incoherentes() (mod.rs) partagée par l'alerte, /structure#cases-incoherentes, Mes salles et la fiche de salle (astérisque rouge).

### Version 2.38.1 — Carte de bande (accueil)

Version Rust 2.38.1 : tuile « Truies inséminées » (nombre + date de la 1re IA) en tête du flux ; l'écho affichée ne peut plus être antérieure à la 1re IA du cycle (une écho « vide » du cycle précédent s'affichait) ; « Truies → verraterie » remplacée par « Truies en verraterie » (truies de la bande logées dans une salle de verraterie, `flux.en_verraterie`) ; emplacements actuels triés par bâtiment puis ordre/nom de salle (Engraissement 1 → 5) au lieu de l'effectif.

### Version 2.38.0 — Caméras des salles

Version Rust 2.38.0 : Mes équipements → Caméras (/equipements/cameras, table `camera`, migration 0023) : nom, adresse RTSP, identifiant/mot de passe gardés sur le serveur (extraits de l'adresse s'ils y sont collés), salle rattachée. Image servie par /salles/camera/{id}/image : ffmpeg tire une image du flux seulement à la demande (une capture à la fois par caméra, resservie 2 s, délai 10 s, rien sur le disque, mot de passe masqué dans les erreurs). La fiche salle affiche l'image et un bouton « Direct » (rafraîchissement 2 s tant que l'onglet est visible). Nécessite le paquet `ffmpeg` sur le serveur.

### Version 2.37.2 — Saisie par groupe

Version Rust 2.37.2 : choix Insémination / Chaleur (radio saisie_type, JS change l'action du formulaire).

### Version 2.37.1 — Paramètres

Version Rust 2.37.1 : retrait du bloc de renvoi « Aliment » de /parametres.

### Version 2.37.0 — Note de rentabilité

Version Rust 2.37.0 : /inseminations?bande= affiche note_rentabilite (mod.rs) sur 20 par truie. Inclut le travail non commité déjà en ligne (tableau des inséminations par bande, module_equipements).

### Version 2.36.2 — Capacités du flux

Version Rust 2.36.2 : conduite::capacites lit d'abord reglage.capacite_* (édité dans Paramètres), parametre.places_* (figé, plus éditable) seulement en repli.

### Version 2.36.1 — Correctif page Aliment

Version Rust 2.36.1 : la section #alimentation de aliment_previsions.html avait été insérée dans le bloc title (invisible) ; déplacée dans le bloc content, test ajouté.

### Version 2.36.0 — Production prévue par bande

Version Rust 2.36.0 : /flux ajoute la colonne Gestantes (capacite_gestante, fenêtre écho → entrée maternité) et le tableau de production par bande (à vendre = porcs × (1 − pertes après sevrage restantes), pertes moyennes des bandes terminées, 5 % par défaut).

### Version 2.35.0 — Aliment regroupé

Plans d'alimentation et réglages `aliment_*` déplacés de /parametres vers /aliment-previsions (section admin #alimentation, routes /aliment-previsions/plan/*, /reglages/maj accepte retour=aliment). Anciennes routes /parametres/aliment/* conservées.

### Version 2.34.1 — Installation de cloudflared expliquée

Version Rust 2.35.0 : la page Accès à distance affiche les commandes d'installation de cloudflared (paquet .deb officiel, lien /usr/local/bin) avec bouton Copier.

### Version 2.34.0 — Accès à distance (tunnel Cloudflare)

Version Rust 2.34.0 : `src/tunnel.rs` — eo-sevel lance et surveille `/usr/local/bin/cloudflared tunnel --no-autoupdate run` (jeton par TUNNEL_TOKEN, `parametre.cf_token/cf_actif/cf_adresse`), relance avec délai croissant, journal gardé en mémoire ; page admin Mes équipements → Accès à distance (/equipements/acces-distance). Remplace l'add-on Cloudflared de Home Assistant.

### Version 2.33.0 — Travail de la semaine validable

Version Rust 2.33.0 : /conduite/semaine — étapes repliables avec pastille « N à faire / terminé », vraies cases à cocher, « Tout cocher », POST /conduite/semaine/valider : `fait` → `travail_fait` (migration 0021, exclu de `truies_etape`) + `acterealise` pour un vaccin/traitement ; `pleine`/`vide` → événement echo ; `annuler`. IA, sevrage, mises-bas : liens vers leurs saisies (un vrai événement est nécessaire).

### Version 2.32.0 — Heure des mesures, vitesse des ventilateurs

Version Rust 2.32.0 : le pont Zigbee (eo-regul) horodate avec `last_seen` de Zigbee2MQTT (option `advanced.last_seen: epoch` activée) — au redémarrage Z2M republiait l'état en cache (PS-3 muet depuis le 26/09 01:14 paraissait mesuré à 06:58). Fiche salle et Mes salles : « Dernière mesure à HH:MM » (date si > 24 h, rouge si > 1 h) ; capteurs de l'automate avec valeur et heure ; ventilateurs : vitesse, arrêt, heure (variateur Tuya TS0601 : pas de lecture sur demande, vitesse connue seulement quand elle change).

### Version 2.31.0 — Alertes SMS Free Mobile

Version Rust 2.31.0 : `src/sms_free.rs` (API « Notifications par SMS » de Free : smsapi.free-mobile.fr/sendmsg), boucle lancée par eo-sevel chaque minute : battement `parametre.sms_battement`, SMS au redémarrage si arrêt > 5 min (coupure du 26/09 22h28 → 27/09 ~9h sans alerte), SMS des nouvelles `ambiance_alerte` ouvertes (critiques ou toutes, `sms_free_niveau`) ; page Mes équipements → Free SMS (/equipements/sms, réglages admin, SMS de test, historique `sms_envoi`, migration 0020).

### Version 2.30.1 — Rubrique Vente directe

Version Rust 2.30.1 : le menu « Achats et ventes » de la 2.30.0 devient « Vente directe » (vente à la ferme seule, visible si module_vente_directe) ; Économie et imports reviennent dans Ma gestion.

### Version 2.30.0 — Prestataires : bandes confiées, inventaire, aliment, pesées

Version Rust 2.30.0 : module `src/routes/prestataire.rs` (portail /engraissement réécrit) ; migration 0019 (`prestataire_option.equarrissage_obligatoire`, historique `bande_prestataire`, `consoaliment_bande` livraison/relevé, `pesee_bande`, réglage `debut_pesees`=180) ; `declarationmort.num_equarrissage` exigé selon l'option du prestataire de la bande ; inventaire via `mouvementstock` (est_stock=1) ; calendrier des stades par bande depuis les réglages de conduite, GMQ et date estimée du poids cible ; bandes confiées depuis Utilisateurs et depuis la fiche bande (`confier_bande`, un seul prestataire à la fois).

### Version 2.29.0 — Calendrier : IA après sevrage, vaccins, lisibilité

Version Rust 2.29.0 : (1) étape « IA des truies sevrées » le lundi qui suit le sevrage (IA du lundi au mercredi chez Emmanuel), liste des truies sevrées sans IA depuis ; (2) vaccins et traitements des protocoles truies (`acteprotocole`, référence gestation = IA, mise-bas, sevrage) placés pour chaque bande, « fait le … » depuis `acterealise` ; `Etape` porte désormais des chaînes possédées et est complétée par `toutes_etapes` ; (3) étiquettes du calendrier lisibles (texte blanc forcé, plus grand). Après le sevrage les truies quittent la bande : les listes verraterie/IA sevrées les retrouvent par leur événement de sevrage.

—

### Version 2.28.0 — Calendrier de conduite : parcours des truies

Version Rust 2.28.0 : demande d’Emmanuel (« il manque les IA, passage verraterie… »). Trois étapes de plus dans `conduite::etapes` : contrôle des retours en chaleur (IA + 21 j), passage en gestante (IA + écho + 2 j ; truies écho pleine encore en verraterie), passage en verraterie au sevrage (truies sevrées encore en maternité). Le réalisé (IA faites, mises-bas, truies sevrées) s’affiche dans les cases du calendrier et la liste de la semaine.

—

### Version 2.27.0 — Supprimer une case occupée

Version Rust 2.27.0 : Structure → Supprimer une case même occupée ou avec un historique : on choisit la case qui reprend tout. Transferts, pertes, inventaires, réceptions, observations, nourrices, événements et truies sont rattachés à la case choisie, puis un inventaire de reprise daté du jour fixe l’effectif exact (porcs repris + présents), sans quoi les transferts rattachés fausseraient `case_pig_count`. Transaction unique.

—

### Version 2.26.4 — Saisie rapide « Perte porcelet »

Version Rust 2.26.4 : le bouton de saisie rapide « Perte porc et porcelet » (perte en maternité, sur une portée) devient « Perte porcelet » ; « Perte porc » (bande et case) inchangé.

—

### Version 2.26.3 — Mes salles rangées par stade

Version Rust 2.26.3 : Mes salles suit le parcours des animaux, avec un titre par stade : verraterie, gestante, maternité, post-sevrage, post-engraissement, engraissement, autres (`salles::stade`, type puis nom ; tri naturel des numéros).

—

### Version 2.26.2 — Bande suivie dans les transferts de case à case

Version Rust 2.26.2 : un transfert de porcs « case → case » était enregistré sans bande_id ; la salle d’arrivée affichait « aucune bande en place » (Post-Engraissement : 313 porcs de B2.12 ; PS 2 et PS 3 : B3.19). Corrigé à la saisie (`bande_de_la_case`), à l’affichage (Mes salles remonte la chaîne des transferts, mouvements trouvés aussi par les cases) et dans les données (réparation idempotente au démarrage dans `achever_init` : 5 transferts au 26/09).

—

### Version 2.26.1 — Champs imposés par une courbe grisés

Version Rust 2.26.1 : fiche de salle et EO_Awel — quand une courbe est choisie, consigne fixe et ventilation mini/maxi (que la courbe impose via `consigne_courbe`) passent grisés en lecture seule, mention « fixé par la courbe ». readOnly et non disabled : la valeur fixe reste envoyée et conservée.

—

### Version 2.26.0 — Surveillance électrique, ventilation et réglages dans la fiche de salle

Version Rust 2.26.0 : (1) Surveillance électrique (`/ambiance/zigbee/electricite`, migration 0018) : pour chaque compteur Zigbee, marche/arrêt au-dessus d'un seuil de puissance, périodes de marche (`zigbee_marche`), temps de marche et démarrages du jour, kWh depuis le relevé de 6 h, phases (V, A), courbe 24 h (puissance historisée toutes les 2 min), alertes coupure de phase / surpuissance / marche trop longue / arrêt trop long, calculées par eo-regul. (2) Fiche de salle : vitesse de ventilation réelle (sorties rôle ventilation : IPX lu en direct, Zigbee en base) et consigne calculée ; formulaire consigne fixe ou courbe, plage, ventilation mini/maxi, seuils d'alerte (`/salles/{id}/reglages`, upsert `ambiance_salle`). Les seuils de la salle s'appliquent aussi aux sondes Zigbee sans seuils propres. (3) Champs d'alerte qui débordaient des cartes Zigbee : corrigé.

—

### Version 2.25.1 — Bandeaux de couleur

Version Rust 2.25.1 : retour d’Emmanuel sur la 2.24.1 (« plus lisible mais moins sympa ») — bandeau dégradé remis volontairement sur les cartes, nom en blanc : Mes salles (une teinte par type de salle), Éclairage (ambre), Zigbee (vert ; rouge si l’appareil est muet).

—

### Version 2.25.0 — IPX800 et Zigbee dans Mes équipements

Version Rust 2.25.0 : demande d'Emmanuel — « chaque appareil sa section ». Menu Mes équipements : entrées Automate IPX800 (/ambiance/automate) et Réseau Zigbee (/ambiance/zigbee). Page /equipements : deux cartes avec l'état lu en base (adresse et nombre de points de l'IPX ; pont Zigbee connecté et nombre d'appareils). Le Zigbee n'est plus un onglet de l'automate.

—

### Version 2.24.1 — Noms de salles lisibles

Version Rust 2.24.1 : les cartes (Mes salles, Éclairage, Zigbee, Ajouter une lampe, étapes du formulaire) utilisaient `<header>`, que style.css peint en bandeau vert foncé avec texte blanc (règle globale de l'en-tête du site) : noms illisibles. Remplacé par `<div class="tete">`. Ne jamais utiliser `<header>` dans le contenu des pages.

— Dernière mise à jour de ce document : 25 septembre 2026.

### Version 2.24.0 — Zigbee

Version Rust 2.24.0 : le réseau Zigbee d'Emmanuel (SLZB-MR5U, 25 appareils) quitte Home Assistant pour Zigbee2MQTT + Mosquitto sur ce serveur (voir mémoire serveur). `eo-regul` embarque un pont MQTT (src/zigbee_pont.rs, crate rumqttc, EO_ZIGBEE_MQTT=hôte:port ou off) : appareils, dernières valeurs (écriture bornée), historique 90 j des grandeurs lentes, file de commandes `zigbee_commande`, alertes pile/muet/seuils de température (migration 0017). Onglet Automate → Zigbee (`/ambiance/zigbee`, src/routes/zigbee.rs) : appareils par type, valeurs, pile, signal ; rattacher à une salle crée des points `ipx_point` hote='zigbee', chemin '<ieee>/<grandeur>' — lus/commandés par le pont dans `lire`/`ecrire` du démon et `envoyer_commande` de l'application. Donc : sondes dans Mes salles (valeurs + frise 14 j si pas de sonde IPX), interrupteurs comme lampes des programmes d'éclairage, compteurs relevés chaque jour dans Eau et électricité.

### Version 2.23.0 — Mes salles

Version Rust 2.23.0 : Ma gestion → Mes salles (`/salles`, `/salles/{id}`, src/routes/salles.rs). Une carte par salle (bande en place et jour, truies, porcs/places, T/H/lux, lampes, alertes, lavage) et une fiche réunissant tout ce qui est relié à la salle : cases et animaux, ambiance et 14 jours d'historique, éclairage et programmes, dimensions et ventilateurs (renouvellement d'air), alertes, mortalités et observations 30 j, mouvements, préconditionnement, points de l'automate. Rien n'est saisi : chaque bloc se construit à partir des liens en base et disparaît s'il est vide (demande d'Emmanuel : « chaque chose qui se relie à la salle s'incrémente ou le contraire »). Accès : droit « structure ».

### Version 2.22.1 — Section actuelle dans la liste des truies

Version Rust 2.22.1 : colonne « Section » sur /truies — salle actuelle (celle de la case si la case est connue, sinon `truie.salle_id`) et nom de la case.

### Version 2.22.0 — Ajouter une lampe en trois clics

Version Rust 2.22.0 : retour d'Emmanuel — déclarer une lumière était incompréhensible (il avait enregistré « Ecl Verraterie » avec le chemin `[1].name` copié de l'explorateur). Nouvelle page `/ambiance/eclairage/lampe/nouvelle` : liste des sorties commandables lues sur l'automate, bouton Allumer/Éteindre pour reconnaître la bonne, nom + salle, Enregistrer ; carte « Mes lampes » sur la page Éclairage (essai, suppression, lampes mal configurées signalées). **Correctif** : les objets `api/core/io/…` se lisent par `on` et se commandent par `{"on": bool}` (application et eo-regul) ; avant, la lecture d'un relais rendait son `_id` et la commande envoyait `{"value": 1}`.

### Version 2.21.0 — Pages de l'automate réorganisées

Version Rust 2.21.0 : les pages IPX800 partagent une barre d'onglets (`ambiance_automate_onglets.html`) : Vue d'ensemble (nouvelle, `/ambiance/automate`, src/routes/automate.rs — état de l'accès et de la régulation, compteurs, étapes de mise en route, dernières commandes, sans lecture en direct), Entrées et sorties (`/ambiance/ipx`), Carte, Éclairage, Explorateur (`/ambiance/ipx/explorateur`) et Accès (`/ambiance/ipx/acces`). L'ancienne page unique `/ambiance/ipx` est découpée par le paramètre `vue` ; « Ajouter » dans l'explorateur ouvre le formulaire d'ajout prérempli. Les adresses existantes restent valables.

### Version 2.20.0 — Programmes d'éclairage

Version Rust 2.20.0 : page `/ambiance/eclairage` (src/routes/eclairage.rs, migration 0016_eclairage.sql). Programmes combinant plages horaires, seuils de luminosité avec hystérésis et délai, minuterie d'extinction ; forçage temporaire, mode essai par programme, pause, duplication, journal (ipx_commande_journal). Décision dans `src/eclairage_core.rs`, pure et testée, compilée dans l'application et dans `eo-regul`, qui l'applique à chaque cycle (passe bornée à 20 s). Le programme agit sur fronts : un geste manuel n'est pas contredit avant le prochain changement. État par sortie dans `eclairage_etat`. Au 24/09, aucun relais d'éclairage n'est encore déclaré pilotable sur l'IPX : à faire depuis la carte de l'automate avant le premier programme.

### Version 2.19.2 — IA toujours rattachées à la bande

Version Rust 2.19.2 : une IA (ou chaleur) saisie sur une truie sans bande — cas normal juste après le sevrage — est rattachée à la bande active dont l'IA prévue tombe à 10 jours près (`bande_pour_ia`). Quand une truie est affectée à une bande (fiche truie, affectation groupée, déplacement depuis la fiche bande), ses IA, chaleurs et échos du cycle en cours restés sans bande y sont rattachés (`rattacher_cycle_a_la_bande`). Données corrigées le 23/09 : 64 IA/chaleurs de B2.33 (30 truies) rattachées ; la bande affichait « 1/1 IA, 100 % » au lieu de 30 IA, 19 pleines (63 %).

### Version 2.19.1 — Saisie rapide « Perte porc » avec case

Version Rust 2.19.1 : le bouton de saisie rapide « Perte porc après maternité » devient « Perte porc » et propose une case facultative (cases de la bande d'abord, via `/api/bande/{id}/cases`, puis les autres). La case est enregistrée dans `declarationmort.case_id` (effectif de la case décompté) et le stade est déduit de la salle.

### Version 2.19.0 — Truie vide, bandes avec truies / sevrées

Version Rust 2.19.0 : bouton « Truie vide : annuler la mise-bas » (maternité, onglet Sevrage, et page de sevrage) sur les mises-bas à 0 né : `POST /bande/{id}/mise-bas/{mise_bas_id}/vide` supprime la mise-bas et ses soins non faits, sort la truie de la bande et journalise ; refusé s'il y a des porcelets, pertes, adoptions ou un sevrage. Page Bandes séparée en « Bandes avec truies » (gestation, maternité) et « Bandes sevrées » (au moins un sevrage et plus aucune portée en cours).

### Version 2.18.4 — Adoptions constatées au sevrage

Version Rust 2.18.4 : sur la page de sevrage, une truie peut avoir plus de sevrés que de présents. Les porcelets en plus sont enregistrés comme adoptions (`adoptionporcelet`, « Adoption constatée au sevrage ») depuis les truies cochées qui en ont en moins, les plus gros écarts d'abord ; ce qui manque encore part en perte avec la cause choisie ; un surplus net de la bande est refusé, sauf choix « Nés vifs sous-estimés » qui corrige la mise-bas. Le tout dans la même transaction que le sevrage.

### Version 2.18.3 — Connexion rétablie

Version Rust 2.18.3 : retour du renvoi classique des formulaires quand la réponse du fetch n'est pas du HTML. Le script global envoie `FormData` (multipart) et les routes `Form` répondent 415 : ce 415 était jusqu'ici silencieusement suivi d'un envoi classique. La 2.18.2 le transformait en erreur affichée, ce qui bloquait tous les formulaires dont la connexion (retirée au bout de quelques minutes, retour en 2.18.1). À faire proprement, avec essai en navigateur : envoyer en `application/x-www-form-urlencoded` hors formulaires de fichiers.

### Version 2.18.2 — Plus aucune saisie perdue sur une erreur

Version Rust 2.18.2 : une erreur d'enregistrement reste affichée sur la page (même si la réponse n'est pas du HTML) ; alerte du navigateur avant de quitter une page dont un formulaire a été modifié sans être enregistré ; la page d'erreur propose « Revenir à ma saisie ». Fiche bande, saisie groupée : une ligne « Sevrés » remplie coche la truie, contrôle avant envoi, et l'action Sevrage renvoie vers `/bande/{id}/sevrage` avec les chiffres saisis préremplis (l'ancien sevrage groupé n'effectuait ni contrôle d'effectif ni sortie de bande).

### Version 2.18.1 — Plans d'alimentation lisibles

Version Rust 2.18.1 : Paramètres → Plans d'alimentation (`src/routes/plans_aliment.rs`) : une carte par catégorie (libellé clair, jour 0 de référence), triée par jour de début, frise des jours (transitions superposées, fin de période attendue d'après la conduite), alerte sur chaque période sans aliment prévu, modification repliée ligne par ligne, ajout avec catégories proposées. Mêmes formulaires et routes qu'avant.

### Version 2.18.0 — Page de sevrage par bande

Version Rust 2.18.0 (`src/routes/sevrage.rs`). `/bande/{id}/sevrage` : une ligne par portée non sevrée (nés vifs, adoptions, pertes déclarées, présents), nombre compté au sevrage prérempli, poids moyen facultatif. Un nombre inférieur aux présents enregistre l'écart en perte (cause choisie) avant le sevrage, dans la même transaction ; supérieur, il est refusé (adoption à saisir). Porcelets vers une case de post-sevrage (facultatif, capacité contrôlée) ; une fois le sevrage confirmé, les truies sortent de la bande (file « Prochaine IA ») et peuvent être déplacées vers une salle (verraterie proposée). Liens depuis la fiche bande, la liste de travail de la semaine et le Rapport.

### Version 2.17.0 — Conduite en bandes : calendrier, places et flux, analyse

Version Rust 2.17.0 (`src/routes/conduite.rs`). `/conduite` : calendrier des bandes semaine par semaine (IA, échos, entrée maternité, mises-bas, sevrages, transferts, départs), charge de travail pondérée par le nombre d'animaux avec semaines calmes / chargées / pics, occupation prévue maternité / post-sevrage / engraissement contre les places réglées. `/conduite/semaine` : liste de travail imprimable, truie par truie, plus soins de portée, protocoles, tâches et entretiens. `/flux` : occupation prévue sur 20 semaines, places de post-sevrage au jour de chaque sevrage, sorties d'engraissement (sevrés − morts − vendus), occupation actuelle par salle. `/analyse-bandes` : comparaison des bandes 1/2/3 sur leurs cycles, impact de la bande la plus faible sur l'année, truies qui tirent une bande vers le bas (petite portée, mort-nés, pertes, retours, faiblesse répétée), homogénéité des lots (étalement des mises-bas, poids de sevrage et de carcasse, étalement des départs). Alertes à 10 jours des étapes et des débordements de places dans la cloche d'alertes et le Rapport. Prévisions : taux de mise-bas et sevrés par portée tirés de l'historique de l'élevage. Places : `parametre.places_*` puis `reglage.capacite_*`.

### Version 2.16.0 — Rapport températures et actions 48 h, régulation

Version Rust 2.16.0 : nouvelle page `/rapport` (bouton « Rapport » du menu), bilan complet de l'élevage sur 7/14/30 jours : synthèse chiffrée, bandes, reproduction, maternité, mortalité (sans double compte des pertes de maternité), sanitaire et délais d'attente, stocks, ventes, eau/électricité, actions à mener dans les 48 h (étapes des bandes, rappels sanitaires, soins de portée, tâches, entretiens, alertes d'ambiance ouvertes, sondes ou régulation muettes) et récapitulatif des températures par salle (actuel, consigne, moyenne/min/max 24 h et 7 j, écart à la consigne, part des relevés hors seuils, alertes). Imprimable / PDF depuis le navigateur ; l'ancienne adresse `/rapport-hebdomadaire.pdf` y renvoie. `eo-regul` ne lève plus l'alerte « ventilation inefficace » quand sa commande n'est pas envoyée (simulation ou salle observée) : c'était la source des fausses alertes critiques. Saisie rapide « Perte porc après maternité » (bande seule) reprise de la version du 21/09. Feuille de style : celle déployée le 21/09 (navigation compacte recentrée, thèmes) plus la console des équipements 2.15.

### Version 2.15.5 — GTTT imprimable sur trois périodes

Version Rust 2.15.5 : `/gttt` reprend la présentation du GTE (bouton Imprimer, A4 paysage, en-tête avec le nom de l'élevage, trois périodes consécutives côte à côte). Les indicateurs ISSF, IDSR et taux de réforme du modèle Cooperl restent à faire.

### Version 2.15.4 — GTE refondu sur le modèle Cooperl (3 périodes + coûts)

Version Rust 2.15.4 : refonte du GTE (`/gte`), à partir des vrais rapports
GTE/GTTT Cooperl fournis par Emmanuel. Changements :
- **Trois périodes comparées côte à côte** (la plus récente d'abord, comme
  le livret GT-Porc) au lieu d'une seule période isolée — la longueur de
  période choisie ci-dessus (3/12/24/36 mois ou dates personnalisées)
  détermine la durée de chacune des trois colonnes.
- **Nouveau bloc « Résultats & Coûts (€ / truie présente / an) »** : produit
  animaux, marge sur coût alimentaire, alimentation, renouvellement (dont
  semence/IA à part), dépenses de santé — calculé sur les factures/ventes
  réellement enregistrées sur l'élevage entier pour chaque période. Ce
  volet économique manquait entièrement à l'ancien GTE, qui n'affichait que
  le technique.
- **En-tête avec le nom de l'élevage** (paramètre déjà saisi ailleurs dans
  l'application), pour ressembler à l'en-tête du rapport Cooperl.
- Les cases à cocher pour masquer un bloc ou une ligne (ajoutées en 2.15.3)
  restent disponibles sur cette nouvelle mise en page.

Le rapprochement avec le GTTT (indicateurs de reproduction : ISSF, taux de
réforme, IDSR…) n'est pas dans cette version — l'essentiel des données
nécessaires (réformes datées, causes de sortie, historique des saillies par
truie) est déjà saisi ailleurs dans l'application ; les relier demandera un
travail spécifique à part.

### Version 2.15.3 — filtres de lignes sur les rapports GTE et coûts de bande

Version Rust 2.15.3 : sur la fiche bande, le « Détail des dépenses » propose
des cases à cocher par catégorie et par fournisseur (ex. exclure un
fournisseur de semence comme Yxia ou DanBred) — le total affiché se
recalcule en direct. Sur le GTE, chaque bloc et chaque ligne d'indicateur
peuvent être masqués individuellement (choix mémorisé sur l'appareil),
pour ne garder que ce qui compte à l'impression ou à la lecture. D'après
les modèles GTE/GTTT Cooperl fournis par Emmanuel : un rapprochement plus
complet du GTTT vers ce format (3 années + référence nationale) est prévu
dans une prochaine version.

### Version 2.15.2 — EO_Treizh, EO_Kren et visuel des équipements

Version Rust 2.15.2 : la station Domino de pesée et de tri porte le nom
EO_Treizh. Ajout d’EO_Kren pour la distribution d’aliment filaire :
passerelle Kren-Link en Ethernet/PoE, nœuds Kren-Node sur bus CAN. Les pages
« Mes équipements » utilisent maintenant le visuel sombre de supervision
testé, sans modifier le thème général du logiciel.

### Version 2.15.1 — seuil de la panne de ventilation corrigé

Version Rust 2.15.1 : l'alerte « Ventilation commandée à X % depuis 20 min
sans effet » (`eo-regul`) se déclenchait dès que la commande dépassait 60 %,
alors que le commentaire du code visait une ventilation « à fond » — à 66 %,
le régulateur a encore de la marge pour pousser plus fort, ce n'est donc pas
forcément une panne de ventilateur, de gradateur ou de trappe : la
température peut simplement mettre plus de 20 minutes à redescendre par
forte chaleur. Le seuil passe à 95 % (commande **et** minimum sur la
fenêtre de 20 minutes) pour ne déclencher cette alerte critique que lorsque
la ventilation tourne vraiment à son maximum sans effet.

### Version 2.15.0 — équipements EO_Awel, EO_Laezh et EO_Treizh

Version Rust 2.15.0 : nouvelle rubrique « Mes équipements » après « Ma
gestion ». EO_Awel regroupe la ventilation existante ; EO_Laezh prépare la
supervision de la machine à lait pour porcelets ; EO_Treizh prépare la
supervision de la station Domino de pesée et de tri. Les deux derniers écrans
signalent clairement qu'ils sont en préparation tant que leurs automates ne
sont pas raccordés.

### Version 2.14.9 — limiter la longueur de l'historique d'une truie, emplacement en lecture seule

Version Rust 2.14.9 : dans la fiche truie, onglet « Historique », un menu
« Afficher » permet de réduire la liste (10 / 20 / 50 derniers événements ou
tout afficher) au lieu de toujours tout dérouler — utile pour les truies avec
beaucoup de rangs. Sur la fiche bande, le tableau « Emplacement actuel »
renomme la colonne en « Case » (c'est déjà la case où se trouvent les porcs)
et retire le formulaire de modification directe de l'effectif, pour éviter
une correction accidentelle en dehors du suivi des mouvements.

### Version 2.14.8 — modes Simple/Complet pour site et salle, comparaison par salle/site

Version Rust 2.14.8 : « Ajouter un site / une zone » et « Ajouter une salle »
proposent maintenant aussi les modes Simple/Complet. Toutes les infos du mode
complet servent désormais à un rapport de densité : la longueur × largeur
d'une salle alimente deux nouvelles cartes, « Comparaison porcs / m² par
salle » et « ... par site », qui fonctionnent même sans secteur/surface
réglés case par case (repère moins précis que la surface utile, mais
disponible dès la création de la salle).

### Version 2.14.7 — modes Simple/Complet à la création d'une case

Version Rust 2.14.7 : le formulaire « Ajouter une case » propose maintenant
deux modes. Simple : Salle, Nom, Places porcs, N° vanne. Complet : en plus,
places truies, RFID, et les repères de surface complets (secteur, logement,
m² ou longueur × largeur, poids, objectif) saisis en une seule fois — plus
besoin de rouvrir « Régler les dimensions et le secteur » juste après la
création.

### Version 2.14.6 — correction de la page blanche sur erreur d'import

Version Rust 2.14.6 : le mécanisme qui affiche les erreurs de saisie sur
place (sans recharger la page ni perdre ce qui était rempli) excluait par
erreur tout formulaire dont l'adresse contient « pdf » ou « csv » — y compris
les formulaires d'**import** de fichiers eux-mêmes (ex. `/economique/import-pdf`).
Une erreur sur ces formulaires (mauvais fichier, trop de PDF sélectionnés...)
provoquait donc une vraie navigation vers une page d'erreur nue, avec perte
du formulaire. Corrigé : seuls les formulaires qui ne postent pas de fichier
(téléchargements) sont désormais concernés par cette exclusion.

### Version 2.14.5 — comparaison porcs / m² par stade

Version Rust 2.14.5 : nouvelle carte sur Structure regroupant, par stade
(verraterie, gestantes, maternité, PS, PE, engraissement), le total des porcs
présents et de la surface utile déclarée sur les cases de ce stade, avec le
ratio m²/porc résultant.

### Version 2.14.4 — surface disponible saisissable à la création d'une case

Version Rust 2.14.4 : le formulaire « Ajouter une case » n'acceptait que les
places porcs/truies ; il fallait ensuite ouvrir le formulaire séparé
« Repères de surface » pour indiquer les m². Ajout d'un champ « Surface
disponible (m²) » directement à la création — secteur/logement restent à
préciser plus tard si le calcul de densité réglementaire est utile.

### Version 2.14.3 — nombre de bandes et part par bande sur les achats génétique

Version Rust 2.14.3 : sur la page Génétique, une facture « Toutes les bandes »
n'indiquait pas sur combien de bandes le montant était réellement réparti.
Ajout du nombre de bandes concernées et du montant réparti par bande.

### Version 2.14.2 — page Génétique recentrée sur les achats réels

Version Rust 2.14.2 : la page « Génétique » affichait un catalogue fournisseur
complet (40 verrats DanBred, référence externe non liée à l'élevage) au-dessus
des lignées suivies. Remplacé par la liste réelle des achats de génétique
(verrats, cochettes, redevances) déjà enregistrés dans Économie — date,
fournisseur, désignation, montant et bandes concernées.

### Version 2.14.1 — frise du cycle sur une seule ligne

Version Rust 2.14.1 : la grille CSS de la frise « Avancement du cycle » (page
d'accueil) était réglée sur 8 colonnes alors que la frise compte 9 étapes
(IA, Écho, Gestante, Maternité, Mise-bas, Sevrage, Transfert, Finition,
Départ abattoir) — la 9ᵉ étape (Départ abattoir) débordait donc toujours sur
une deuxième ligne. Corrigé à 9 colonnes.

### Version 2.14.0 — thème sombre du plan du site

Version Rust 2.14.0 : la page « Plan du site » adopte un thème sombre dédié
(fond bleu nuit, accents cyan), avec une carte par salle affichant
température, consigne, humidité et ventilation en direct, un statut coloré
(Normal / À surveiller / Ventilation en repli), une courbe 24 h construite à
partir de l'historique réellement enregistré, une jauge de ventilation
moyenne et les alarmes actives. Le reste de l'application garde son thème
clair habituel pour l'instant.

### Version 2.13.1 — correctif d'envoi de l'image de fond

Version Rust 2.13.1 : l'import de l'image de fond du plan échouait pour les
fichiers de plus de 2 Mo (limite de requête par défaut). La route accepte
maintenant jusqu'à 6 Mo, comme les autres imports de fichiers de
l'application.

### Version 2.13.0 — image de fond et blocs libres sur le plan

Version Rust 2.13.0 : le plan du site accepte maintenant une image de fond
(votre propre plan de bâtiment, importé depuis l'éditeur) affichée sous les
salles, et des blocs libres positionnables (titre, texte, lien) pour tout ce
qui n'est pas une salle. Un aperçu du plan apparaît aussi sur la page
d'accueil.

### Version 2.12.0 — plan visuel du site

Version Rust 2.12.0 : nouvelle page « Plan du site » (`/structure/plan`), un
plan visuel du bâtiment où chaque salle affiche sa température et son
humidité en direct. La position et la taille de chaque salle se règlent une
fois, à la souris, dans l'éditeur réservé aux administrateurs
(`/structure/plan/editer`) — rien n'est codé en dur, donc la même page sert
n'importe quel élevage : il suffit d'y placer ses propres salles.

### Version 2.11.4 — fiabilisation de l'affichage en direct

Version Rust 2.11.4 : la page Ambiance actualise maintenant aussi ses états
visuels (Normal, À surveiller, Ventilation en repli) et la couleur des tuiles
quand de nouvelles mesures arrivent. La comparaison des codes de bande ne
confond plus des codes distincts ayant les mêmes chiffres.

### Version 2.2.76 — fiabilisation des saisies

Version Rust 2.11.3 : le n° de marquage officiel d'une bande se déduit
maintenant automatiquement du site et du code (ex. site « 35DA9 » + code
« B3.19 » → « FR35DA919 ») et se recalcule à chaque changement de site —
la sélection manuelle dans une liste pouvait laisser deux bandes distinctes
avec le même numéro si l'une changeait de site sans que le numéro suive
(cas réel trouvé : B1.05 et B2.07 partageaient le même numéro). Sur la carte
d'une bande, « Écho confirmée » et « Réussite à l'écho » remontent au début
de la ligne de statistiques (avant « Truies ayant mis bas »), dans l'ordre
chronologique du cycle. Les tuiles Température/Hygrométrie/Ventilation
affichent désormais une petite courbe des dernières 48 h.

Version Rust 2.11.2 : le stade affiché en tête de la carte d'une bande sur
le Centre de pilotage pouvait rester « Lactation » alors que les animaux
étaient déjà physiquement en post-sevrage ou en engraissement (sevrage fait
par un simple mouvement de case, sans passer par l'événement dédié) — le
stade se déduit maintenant en priorité de l'emplacement réel des animaux.
L'encart « Emplacements actuels et ventilation » est aussi déplacé juste
sous le numéro de la bande, au lieu d'apparaître plus bas dans la carte.

Version Rust 2.11.1 : page Ambiance & ventilation refaite (chaque salle
affiche température et ventilation en gros, avec une pastille de couleur
résumant son état — normal, à surveiller, ventilation en repli) ; page
Archives dotée d'un bouton « Désarchiver » qui manquait ; vérification à la
création d'une bande renforcée pour repérer un doublon même si le code est
saisi différemment (ex. « 2.33 » vs « B2.33 »).

Version Rust 2.11.0 : climat (température/hygrométrie/ventilation) affiché
dans le bandeau de chaque bande et par emplacement occupé sur la Conduite
des bandes, nouvelle page d'historique complète par salle avec sélection
libre de la période, et correction d'une borne stricte qui pouvait exclure
à tort une mesure d'un historique par plage de dates.

Version Rust 2.10.3 : le jour de présence en maternité (qui pilote la
descente de la consigne) se déduit désormais de la date de mise-bas réelle
des truies présentes quand aucun transfert de bande n'existe pour la salle
— cas systématique en maternité, où les entrées sont individuelles.

Version Rust 2.10.2 : refus d'activer « Mesurer »/« Piloter » une salle sans
adresse d'automate réellement saisie (le champ affichait une adresse
d'exemple grisée facile à confondre avec une vraie valeur), badge « non
renseignée » si le cas existe déjà, et correction du menu « Sortie
ventilation » qui proposait le chemin de lecture au lieu du chemin
d'écriture d'un point.

Version Rust 2.10.1 : menus déroulants pour le raccordement des salles
(entrées température/hygrométrie/luminosité, sortie ventilation, adresse de
l'automate) — proposent les capteurs déjà rangés dans l'inventaire IPX,
ceux affectés à la salle en tête de liste, au lieu d'un champ texte vide.

Version Rust 2.10.0 : seuils d'alerte température/hygrométrie affichés
directement sur chaque salle de la page Ambiance (au lieu d'être cachés dans
Réglages), et trois graphiques par salle (température, hygrométrie,
ventilation) sur les dernières 48h, tracés dans le navigateur à partir de
l'historique déjà enregistré, avec les seuils réglés en pointillés sur la
courbe.

Version Rust 2.9.2 : correction du contrôle de restauration de sauvegarde,
qui comparait à tort contre le schéma de la base *actuelle* au lieu du socle
historique stable — toute sauvegarde antérieure à une nouvelle table ajoutée
par une mise à jour depuis était refusée à tort, message désormais précis
sur la raison exacte du refus.

Version Rust 2.9.1 : liste déroulante de capteurs analogiques connus (TC100,
TC4012, LS100, TC5050, RH-SH100, pinces X400CT, X200PH, X200ORP) qui
préremplit la formule de conversion sur la carte de l'automate et
l'Explorateur.

Version Rust 2.9.0 : formule de conversion texte (ex. sonde TC100) pour les
entrées analogiques enregistrées via la carte de l'automate ou l'Explorateur
— X représente la valeur brute, opérateurs + - * / ^ et parenthèses acceptés,
priorité sur coefficient/décalage, jamais de valeur fabriquée si la formule
est invalide.

Version Rust 2.8.0 : dimensions de salle (longueur/largeur/hauteur) avec
ventilateurs déclarés (position, fonction extraction/insufflation, voie,
débit) pour un taux de renouvellement d'air calculé plutôt que deviné,
observations comportementales (cannibalisme, queue/oreille mordue) croisées
avec le climat du jour par salle dans une nouvelle page « Comportement &
climat », panneau « Éclairage » sur la page Ambiance (on/off manuel par
sortie IPX déclarée, sans extinction automatique pour l'instant) — avec
correction du garde-fou de régulation, qui protège désormais la seule sortie
ventilation réécrite par eo-regul au lieu de bloquer toute commande dans une
salle pilotée — et préconditionnement des salles : hypothèse d'arrivée d'une
bande par salle qui démarre le préchauffage (jour 0 de la courbe) avant le
transfert réel, avec priorité au vrai transfert et expiration automatique
après 48h sans arrivée.

Version Rust 2.7.0 : classement des objets de l'automate par nom (motifs de
section), capacités des bâtiments sans valeur préremplie.

Version Rust 2.6.0 : carte de l'automate — les entrées et sorties rangées par
bloc matériel, affectation d'une salle en un geste, commande des sorties.

Version Rust 2.5.1 : explorateur IPX corrigé sur installation réelle — filtrage
par objet, liste non tronquée, gabarits d'API conformes au firmware V5.

Version Rust 2.5.0 : inventaire nommé des entrées et sorties de l'automate,
commande manuelle d'une sortie, et relevé automatique quotidien des compteurs
d'eau et d'électricité vers le suivi existant.

Version Rust 2.4.0 : entrée de pharmacie par scan du Data Matrix de la boîte
(produit reconnu, lot et péremption relevés automatiquement, apprentissage des
codes au premier scan).

Version Rust 2.3.1 : page de diagnostic de l'automate IPX800 (état en direct,
relevé des identifiants, réglage de l'accès depuis l'interface).

Version Rust 2.3.0 : le logiciel devient EO_Sevel et intègre l'ambiance et la
régulation de ventilation (démon eo-regul séparé, consigne suivant l'âge de la
bande, repli de sécurité sur mesure douteuse, historique replié). Rappel de la
version précédente — correction du sevrage rapide (enregistrement atomique,
effectifs des cases, contrôles des doublons et des destinations), des sorties
pharmacie liées aux traitements et des alertes calculées depuis leur date de fin.
Les transferts de truies futurs ou antérieurs à un transfert enregistré sont
refusés. Docker embarque les ressources nécessaires et utilise Cargo.lock.
Neuf tests de non-régression couvrent les scénarios corrigés.

Pour un traitement ancien non traçable ou après un inventaire, l'option
« Corriger uniquement l'historique » permet une régularisation manuelle du stock.

La vérification des imports a identifié trois anomalies encore à traiter :
perte de lignes répétées lors du remplacement d'une facture, collision de
références entre fournisseurs et conservation de l'ancienne bande lorsqu'un
apport remplacé est explicitement laissé sans bande.

### Version 2.2.75 — traitement sur plusieurs jours et sortie de stock automatique

Le formulaire « Traitement » de la fiche truie (utilisé entre autres pour la
synchronisation des chaleurs à l'Estrogène / P.G. 600, sur 18 jours) n'avait
qu'une seule date. Il a maintenant une date de début et une date de fin,
pré-remplie automatiquement à 18 jours après la date de début (modifiable).

Autre point soulevé : l'inventaire quotidien de la pharmacie n'était pas mis
à jour pour ces traitements longs, obligeant à sortir le stock à la main
chaque jour. Désormais, dès l'enregistrement du traitement, une dose par
jour de la période est automatiquement sortie du produit concerné dans le
stock pharmacie, avec un mouvement daté et noté (« Traitement du … au …,
1 dose/j ») — un seul geste couvre les 18 jours. Si le stock est
insuffisant, il est ramené à 0 (jamais négatif) et l'écart est signalé dans
les journaux du serveur pour être corrigé au prochain inventaire ; la
saisie du traitement elle-même n'est jamais bloquée pour autant.

### Version 2.2.74 — liste de suggestions pour le type de salle

Le champ « Type » du formulaire « Ajouter une salle » (Structure) était un
texte libre, sans liste. Il propose maintenant les types reconnus par
l'application (Verraterie, Maternité, Post-sevrage, Engraissement,
Quarantaine, Local technique) via une liste de suggestions, tout en
restant modifiable librement — un élevage peut avoir un type de salle qui
ne figure pas dans cette liste. Les quatre premiers sont ceux dont
l'application déduit automatiquement le stade des cases qu'ils contiennent.

### Version 2.2.73 — retours de terrain sur la réussite à l'écho et le cycle

Le repli ajouté en 2.2.72 ne se déclenchait que si `cs_truies_saillies` était
renseigné en base ; sur une bande où ce compteur est vide (import partiel,
bande créée directement dans l'appli), le taux retombait quand même sur les
seuls événements IA individuels, encore minoritaires sur le terrain. Un
dernier repli utilise maintenant l'effectif réel de la bande quand ni les
IA individuelles ni le compteur historique ne donnent de dénominateur
fiable. Le calcul distingue aussi désormais pleines et vides.

L'encart réussite à l'écho, qui occupait une section à part en haut de la
fiche bande, est déplacé dans la rangée de petites cases techniques
(Truies actuelles, Portées, Nés vifs…) où il était attendu.

Sur l'accueil, la frise de la bande restait bloquée sur « Échographie »
pendant toute la gestation, de l'écho jusqu'à l'entrée en maternité. Un
repère « Gestante » est inséré entre les deux : l'écho reste un contrôle
ponctuel, la bande devient gestante juste après. L'export calendrier
(.ics) utilisait le jour de l'étape comme identifiant unique — deux étapes
au même jour auraient produit le même événement en double ; il utilise
maintenant la position de l'étape.

Enfin, une bande active sans date de mise-bas renseignée (contrôle qualité
d'Archives) est maintenant complétée automatiquement dès qu'une IA existe
pour elle : date = moyenne des IA de la bande + durée de gestation réglée
dans Paramètres. Exécuté au démarrage du serveur pour les bandes déjà
concernées, et à chaque nouvelle IA enregistrée pour les suivantes. Une
date déjà renseignée n'est jamais modifiée.

### Version 2.2.72 — taux fiable sur les bandes historiques, réussite par emplacement

Le taux de réussite à l'écho affichait 1 IA / 1 pleine sur une bande de 30
truies reprise de l'historique 1.65 : ces bandes n'ont jamais eu d'IA saisie
individuellement, seul le compteur global `cs_truies_saillies` existe. Le
calcul se rabat désormais sur ce compteur quand aucune IA individuelle
n'est enregistrée, plutôt que d'afficher un taux basé sur une poignée de
saisies récentes.

Nouveau : la page Structure affiche pour chaque case/vanne son taux de
réussite à l'écho, toutes truies et toutes bandes confondues — y compris
celles réformées ou déplacées depuis. La case est fixée sur l'événement au
moment de la saisie et ne bouge plus ensuite, ce qui permet
de repérer un emplacement où les truies reviennent souvent vides
(ventilation, courants d'air…) sans attendre que l'IA soit tracée
individuellement.

### Version 2.2.71 — réussite à l'écho visible sur l'accueil

L'encart de la 2.2.70 n'existait que sur la fiche bande ; il fallait l'ouvrir
pour le voir. La carte de chaque bande sur l'accueil affiche désormais aussi
la date et le résultat de la dernière écho enregistrée, ainsi que le taux de
réussite. Le calcul est partagé entre l'accueil, la liste des bandes et la
fiche bande via une seule fonction (`band_reproduction_stats`), au lieu
d'être dupliqué.

### Version 2.2.70 — taux de réussite à l'écho sur la fiche bande

La fiche bande affiche désormais un encart en haut de page : truies
inséminées (IA), truies confirmées pleines à l'écho, et le taux de réussite
correspondant. Il se met à jour dès qu'une écho positive est enregistrée
via la saisie groupée. Le stade « Gestation » affiché sur l'accueil pour la
bande, lui, dépendait déjà d'une écho confirmée positive — il fonctionne de
nouveau depuis la correction du bug de la 2.2.69.

### Version 2.2.69 — saisie groupée « Écho pleine/vide/douteuse » réparée

Même famille de bug que la 2.2.63 : l'action groupée « Appliquer aux truies
cochées » sur la fiche bande (Écho — pleine/vide/douteuse, sevrage) écrivait
dans `evenement` sans préciser `suivi_actif`, en comptant sur le `DEFAULT 0`
du schéma. Les bases réelles, héritées de la 1.65 ou créées avant l'ajout de
ce défaut, portent la colonne sans lui — `CREATE TABLE IF NOT EXISTS` ne
retouche jamais une table déjà existante. L'écriture échouait donc sur le
terrain (`NOT NULL constraint failed: evenement.suivi_actif`) tout en passant
les tests. La colonne est désormais fournie explicitement, comme partout
ailleurs dans le code.

### Version 2.2.68 — accès public en mode démo clarifié

Le calcul des pages accessibles sans connexion mélangeait `&&` et `||` sans
parenthèses : en mode démo, seule `/login` restait réellement publique, le
reste de la condition étant silencieusement neutralisé par la priorité de
`&&`. Le comportement obtenu était correct mais totalement illisible — un
prochain ajout de chemin public aurait pu casser cette hypothèse sans que
personne ne s'en aperçoive. La condition est réécrite explicitement, avec un
commentaire qui documente l'intention (le tunnel de commande en vente
directe est fermé en mode démo).

Nettoyage annexe : `/administration` figurait dans la liste des chemins
réservés aux administrateurs alors qu'aucune route de ce nom n'existe — entrée
morte retirée.

### Version 2.2.67 — la dictée confrontée au terrain

Premières dictées en bâtiment, et trois défauts que seul l'usage pouvait
montrer.

Le moteur ne connaissait pas le vocabulaire : « porcelets écrasés » ressortait
en « pour se les écraser », « par la truie » en « sur le trouille ». Une amorce
de contexte lui est désormais donnée — truie, cochette, porcelets, mise bas,
sevrage, écrasés, chétif, splayleg — ce qui oriente le décodage vers les mots
qu'on attend réellement.

La transcription d'une suite de chiffres ajoute ou perd un chiffre : six
chiffres dictés ressortaient en sept ou huit. Exiger une correspondance exacte
revenait à rejeter une dictée pourtant compréhensible. Le numéro entendu est
maintenant rapproché des numéros réels du cheptel, fenêtre par fenêtre : les
truies à deux chiffres près sont proposées, et l'éleveur tranche d'un tap. Rien
n'est choisi d'office — désigner une truie au hasard serait pire que de dire
qu'on n'a pas compris.

Enfin, transcrire demande une dizaine de secondes sur une machine à deux cœurs,
et l'écran ne le disait pas : la barre défilait sans repère, ce qui passe pour
une panne. Un compteur affiche maintenant les secondes écoulées, et au bout
d'une minute la requête est abandonnée avec un message plutôt que de laisser
attendre indéfiniment.

### Version 2.2.66 — la saisie rapide reprise à partir du terrain

Neuf champs pour déclarer deux porcelets écrasés : la saisie rapide demandait
ce que le serveur savait déjà. Le stade se déduit de la bande ou de la case,
l'âge se calcule depuis la mise-bas — les deux champs disparaissent. Restent la
truie, le nombre, la cause et la date ; la case d'origine et le poids, utiles
hors maternité, passent sous un dépliant. La case d'origine ne propose plus que
les cases où la bande se trouve vraiment, au lieu de toutes celles de
l'élevage : trois entrées à lire plutôt que cinquante.

L'observation en texte libre quitte la perte et le sevrage. Elle ne ressort
dans aucune analyse, et le principe du logiciel est que tout puisse être étudié
ensuite ; un champ que rien ne relit allonge l'écran sans rien apporter.

L'âge des pertes, désormais toujours calculé, l'est plus solidement : une truie
qui a changé de bande depuis sa mise-bas ne fait plus perdre l'information.
C'est lui qui permet d'étudier à quel moment de la lactation les pertes
surviennent.

À la mise-bas, la délivrance se répond par deux boutons Oui et Non au lieu d'un
menu déroulant — un geste au lieu de trois — et la surveillance devient un
interrupteur qui se voit. Un champ splayleg apparaît, avec ce qu'il faut savoir
écrit à côté : ces porcelets sont vivants, ils sont déjà comptés dans les nés
vifs et ne doivent pas en être retirés. Le nombre est conservé sur la truie, où
il sert de critère de sélection des mères à cochettes.

Un mouvement peut désormais porter sur une truie seule, et non plus seulement
sur une bande ou une case : c'est le cas le plus courant en verraterie comme en
maternité. La recherche, jusque-là absente de cet écran, filtre les trois à la
fois.

Nouvelle entrée « Nouvelle cochette » : une cochette entre au cheptel avec son
numéro de travail, son numéro national, sa RFID, sa date de naissance, son
poids d'entrée, sa race, sa lignée, son père et sa mère. Ces informations ne se
retrouvent plus après coup alors qu'elles conditionnent l'analyse de toute sa
carrière. Le poids rejoint les mesures de la truie, un numéro déjà porté est
refusé.

La conduite des bandes ne se rappelle plus en haut de la page Bandes : elle se
règle dans Paramètres, où elle a toujours vécu.

### Version 2.2.65 — savoir que ça écoute, et où ça en est

Retour de terrain sur la dictée : l'attente était aveugle. On ne voyait ni que
le micro captait, ni que le serveur travaillait, et quelques secondes de silence
passent pour une panne — l'éleveur appuie une seconde fois et perd sa dictée.

Le panneau montre maintenant un chrono qui avance et une barre de niveau
sonore : un micro coupé donne une barre plate, ce qu'aucun message ne dirait
aussi vite. Le téléphone vibre au début et à la fin de l'écoute, ce qui compte
quand l'écran n'est pas sous les yeux. L'analyse annonce ses étapes — envoi,
puis transcription — avec une barre qui défile, et l'enregistrement final fait
de même. Au-delà de vingt secondes la dictée s'arrête seule : ce n'est plus une
dictée, et transcrire du bruit de bâtiment ne fait qu'allonger l'attente.

La transcription utilise désormais tous les cœurs de la machine au lieu des
quatre que whisper.cpp prend par défaut, et sa durée est journalisée : c'est la
seule mesure qui permette de décider s'il faut un modèle plus léger. Le réglage
`EO_VOCAL_THREADS` permet de la borner sur un serveur partagé.

Le bouton micro se place enfin au bout de la barre du haut, à droite.

### Version 2.2.64 — la dictée depuis le téléphone

La brique serveur de la 2.2.63 n'avait pas d'interface : il fallait passer par
l'API pour dicter. Un bouton micro apparaît maintenant en haut à droite de
l'écran, sur téléphone uniquement — c'est en bâtiment, les mains prises, qu'on
dicte ; au bureau le clavier va plus vite.

Un appui ouvre l'écoute, un second l'arrête et lance l'analyse. L'écran de
relecture montre alors la truie reconnue, sa bande, la date de mise-bas et
surtout l'effectif encore présent sous la mère : annoncer trois écrasés sur une
portée qui n'en compte plus qu'un se voit avant d'enregistrer, pas après. Le
texte transcrit est rappelé en petit, ce qui permet de distinguer d'un coup
d'œil une erreur de transcription d'une erreur d'analyse.

Tout reste modifiable avant validation — truie, nombre, cause, date — parce que
corriger un champ doit rester plus rapide que redicter. Quand le numéro compris
n'existe pas au cheptel, les numéros voisins sont proposés en boutons : un tap
suffit. Rien n'est enregistré tant que le bouton n'a pas été pressé, et
l'enregistrement passe par les mêmes contrôles que la saisie manuelle.

Le micro exige une connexion sécurisée : en http simple, le navigateur refuse
l'accès et le panneau le dit clairement plutôt que d'échouer en silence.

### Version 2.2.63 — saisie vocale, et pertes de porcelets réparées

Les pertes de porcelets saisies depuis le panneau rapide échouaient sur une
page d'erreur renvoyant au journal. La colonne `declarationmort.horodatage` est
NOT NULL ; le schéma neuf lui donne une valeur par défaut, mais les bases
héritées de la 1.65 portent la colonne sans défaut, et `CREATE TABLE IF NOT
EXISTS` ne redéfinit jamais une table déjà présente. L'écriture qui s'en
remettait au défaut passait donc tous les tests et échouait sur une base
d'élevage réelle — pour la saisie rapide comme pour la déclaration de mortalité
en engraissement. L'horodatage est désormais écrit explicitement, et un test
recrée la table telle que la version Python la créait.

Première brique des ordres à la parole : l'application mobile enregistre
l'énoncé, le serveur le transcrit et le comprend, l'éleveur relit à l'écran, et
rien n'est enregistré avant sa validation. Un ordre ressemble à « truie cinq
zéro zéro trois deux cinq, deux porcelets écrasés » — les numéros se dictent
chiffre par chiffre, la forme qui résiste le mieux au bruit d'un bâtiment.
L'analyse est déterministe, sans modèle de langue : elle ne peut pas inventer un
numéro qui n'a pas été prononcé, et les causes proposées sont uniquement celles
configurées dans l'élevage. Quand le numéro compris n'existe pas au cheptel, les
numéros voisins sont proposés plutôt que la saisie rejetée. La validation passe
par les mêmes contrôles que la saisie manuelle. Les enregistrements audio sont
conservés dix jours pour diagnostiquer les dictées mal comprises, puis effacés ;
le texte transcrit reste. La transcription (whisper.cpp) est optionnelle : sans
elle, l'analyse d'un texte déjà transcrit fonctionne.

### Version 2.2.62 — barre du bas réellement visible sur téléphone

La barre de navigation du bas était bien fixée à l'écran, mais un seul élément
trop large — un tableau non encadré, par exemple — suffisait à élargir la page :
le navigateur affichait alors tout dézoomé et ancrait la barre en bas d'une zone
plus grande que l'écran, obligeant à descendre jusqu'en bas pour la voir. Les
tableaux qui n'étaient pas déjà dans un cadre défilant le sont désormais, et la
page ne peut plus dépasser la largeur de l'écran sur téléphone.

### Version 2.2.61 — navigation refondue avant les essais éleveurs

Neuf menus déroulants deviennent quatre rubriques nommées comme on travaille :
Aujourd'hui, Mes animaux, Mes résultats, Ma gestion. Les cinq écrans d'analyse
(GTTT, productivité, GTE, IFIP, retours d'abattoir) sont réunis sous une seule
porte. Les menus s'ouvrent au clic et non plus au survol — le survol est
intenable au doigt et invisible au clavier ; Échap et le clic à côté referment.
Chaque page indique désormais où elle se trouve : rubrique soulignée, fil
d'Ariane, et onglets des pages voisines sous le titre. Chaque entrée de menu
porte une ligne d'explication, ce qui lève le jargon (GTTT, GTE, IFIP) pour qui
découvre le logiciel. Sur téléphone, la navigation descend sous le pouce avec une
barre fixe en bas dont le bouton central ouvre la saisie rapide. La recherche est
sortie du fouillis, et le menu du compte réunit enfin mot de passe, aide et
administration. Des tests interdisent désormais un lien de menu sans page et une
section d'accès sans entrée de menu.

### Version 2.2.60 — imports de factures et de ventes plus tolérants

Un PDF illisible n'entraîne plus le rejet des autres fichiers du même dépôt, et
une ligne incomprise n'empêche plus d'importer les lignes voisines : les lignes
écartées sont listées, à saisir à la main si besoin. Une référence déjà
enregistrée n'est plus une impasse : la ligne propose le remplacement, à cocher
document par document, ce qui permet de reprendre une facture corrigée par le
fournisseur. Un fichier dont l'aperçu a expiré ou été annulé peut être redéposé.
Nouveauté : un import CSV des factures tous secteurs (aliment, vétérinaire,
semence, génétique) sert de porte de sortie quand le PDF du fournisseur n'est pas
reconnu, avec le même aperçu et le même choix des bandes. Les PDF scannés passent
par la reconnaissance de caractères lorsque poppler-utils et tesseract-ocr sont
installés sur le serveur. La frappe des synthèses Uniporc n'est plus figée à la
forme DA000 et les valeurs sont lues derrière leur libellé plutôt qu'à une
position de colonne. Enfin, l'import économique suit désormais les mêmes droits
que la saisie économique manuelle.

### Version 2.2.59 — GTE imprimable et saisies sans rechargement

La page GTE s'imprime en A4 paysage : tableaux déroulés en entier, en-têtes de
colonnes répétés sur chaque page, filtres de période remplacés par un rappel des
dates. Partout ailleurs, l'enregistrement d'une modification ne recharge plus la
page d'un coup : le contenu est remplacé sur place et la position de lecture est
conservée. Une erreur n'ouvre plus une page séparée, elle s'affiche dans un
encart en haut de la page en cours.

### Version 2.2.58 — synthèse GTE à la structure du livret GT-Porc

Le filtre de période ajouté en 2.2.56 s'applique au détail par lot. La 2.2.58
ajoute au-dessus la synthèse que publie le livret GT-Porc : l'élevage entier
sur la période, en quatre blocs (résultats techniques, post-sevrage,
engraissement, sevrage-vente), avec la référence nationale de l'orientation
active et l'écart. C'est cette présentation qui rend la comparaison possible.

La synthèse réutilise les bornes du sélecteur existant plutôt que d'en poser
un second — deux sélecteurs sur le même écran donneraient deux réponses à la
même question. Sans bornes saisies, elle prend les douze mois glissants et le
dit : une mise à l'année sur une période indéterminée n'aurait aucun sens.

Deux précautions assumées : les indicateurs IFIP « technique » et
« standardisé » reposent sur des courbes de croissance non publiées et ne sont
pas reproduits — tiret et raison affichés, plutôt qu'un chiffre faux qui
invaliderait la comparaison ; les poids vifs sont déduits des poids carcasse
au rendement forfaitaire de 76,5 % retenu par le livret lui-même, modifiable
par le réglage `rendement_carcasse_pct`. Les indicateurs qui demandent une
pesée au changement de salle affichent la même mention et se rempliront seuls
si ces pesées sont saisies un jour.

Un vrai bug trouvé en confrontant les ordres de grandeur aux références : la
consommation d'aliment par truie utilisait l'aliment total de l'élevage et
sortait à plus de 7 000 kg contre une référence à 1 258 — le livret ne compte
à cette ligne que l'aliment des truies. Le calcul passait tous les tests
unitaires ; seule la comparaison l'a révélé. Un test fige désormais l'écart
maximal admis sur sept lignes.

### Version 2.2.57 — historique GTE conservé

- Les bandes archivées restent visibles dans la GTE dès qu’elles possèdent des ventes, des charges d’aliment, des achats ou des effectifs.
- Le filtre de période continue de s’appliquer aux chiffres historiques sans masquer les lots clôturés.

### Version 2.2.56 — périodes GTE, accès et mortalité fiable

- Résultats GTE filtrables entre deux dates, avec raccourcis sur les 3, 12, 24 et 36 derniers mois.
- Présentation clarifiée des rôles et pages visibles de chaque utilisateur.
- La mortalité sous la mère reste indéterminée avant un sevrage réellement enregistré, au lieu d’afficher 100 % à tort.

### Version 2.2.55 — découverte du serveur sur le réseau local

Le serveur s'annonce en mDNS/DNS-SD (`_eosuivi._tcp.local.`) avec son port, sa
version et le nom de l'élevage. Prépare l'application mobile : beaucoup
d'élevages n'ont aucun accès internet — ni nom de domaine, ni certificat, ni
DNS — et le téléphone doit trouver le serveur seul sur le réseau. L'annonce
n'est jamais bloquante (multicast filtré ou interface absente n'empêchent pas le
démarrage) et se coupe avec `EO_MDNS=0`.

### Version 2.2.54 — progression des bandes et commandes clients

- Statistiques de mise bas par bande : truies ayant mis bas, porcelets nés et moyenne par truie.
- Flèche d’avancement, stades réels de Gestation, Lactation, Post-sevrage et Engraissement, avec bâtiment et salle des animaux.
- Rappel visible du lien et du code permettant au client de modifier sa commande de vente directe.

### Version 2.2.53 — accès utilisateurs, bandes et ventilation économique

- Choix du rôle et des pages visibles pour chaque utilisateur.
- Distinction entre bandes configurées et cycles ouverts sur le tableau de bord.
- Import des factures d’aliment bloqué tant que le site manque ou que le stade ne peut pas être reconnu.

### Version 2.2.52 — coût d'achat en GTE, lignées et import génétique

- GTE : le coût d'achat des animaux reçus (`receptionachat`) est imputé au lot
  comme charge d'entrée. La MSA, qui ne retient que l'aliment, surestimait la
  marge des profils post-sevreur et engraisseur ; la marge brute par truie
  repose désormais sur la marge après cette charge. Sans réception d'achat,
  rien ne change pour un naisseur ou un naisseur-engraisseur.
- Truies rattachables à une lignée du catalogue génétique (`truie.lignee_id`,
  colonne additive) au lieu de la seule race en texte libre, qui est conservée.
  Une lignée encore rattachée à des truies ne peut plus être supprimée.
- Import CSV en masse des factures génétiques : modèle téléchargeable, aperçu
  ligne à ligne et application transactionnelle, comme les imports existants.
- Correction de deux erreurs `clippy` (`nonminimal_bool`) qui faisaient échouer
  l'intégration continue sur Rust stable, donc bloquaient toute fusion.

### Version 2.2.51 — identité visuelle et sécurité des accès

- Identité EI ORY Emmanuel intégrée à la connexion, à l’icône du site et à la barre de navigation, avec adaptation mobile.
- Sessions révoquées après désactivation, changement de droits ou changement de mot de passe.
- Commandes publiques empêchées d’écraser une fiche client à partir du seul numéro de téléphone.
- Exposition HTTP de Docker Compose limitée à la machine locale en l’absence de proxy HTTPS.

### Version 2.2.50 — surfaces des cases et fiabilité des cycles

- Dimensions intérieures ou surface utile par case, secteur, mode de logement, poids de contrôle, objectif et diagnostic documenté de charge.
- Effectifs de portée recalculés par cycle réel, historique de maternité continu et contrôles renforcés sur pertes, adoptions et sevrages.
- Affectations économiques, factures et ventilation des ventes consolidées.

### Version 2.2.49 — économie fictive sur cinq ans

Le portail de démonstration complète son historique pour couvrir cinq ans de
ventes et conserve les sept bandes actives espacées de 21 jours. Il ajoute les
ventes, aliments par stade, frais sanitaires, semences et génétique avec leurs
affectations. Les données restent fictives et ne représentent pas un coût de
production complet. Ajout transactionnel unique, sans réinitialiser les comptes
ni remplacer les saisies existantes ; les bandes déjà renseignées sont exclues.

### Version 2.2.43 — ergonomie et gestion quotidienne

- Modification des mises-bas dans la fiche truie, pertes associées synchronisées et effectifs contrôlés.
- Six critères de sélection, tétines/splayleg, mères choisies surlignées et quatre niveaux de réforme.
- Tâches, réparations et entretien réunis ; notes modifiables et supprimables.
- Choix de semence depuis les factures importées ; relevés d’eau limités au site et bandes modifiables.
- Inventaire CSV des produits et silos avec import atomique et contrôle du stock de référence.
- Page Effectifs retirée ; bandes fantômes v1.14 archivées avant nettoyage (liens d’adoption protégés).
- Plan sanitaire, inséminations et pilotage technique réorganisés.

### Version 2.2.42 — maternité et nourrice artificielle

Adoptions atomiques vers une truie ou une case de nourrice, suivi des lots avec pertes
et sevrage vers le post-sevrage, compteurs de vivants et décès séparés des mort-nés et
momifiés. Les cases peuvent être renommées et leur plafond de porcelets est retiré.
Nouvelles tables additives `adoptionporcelet` et `sortienourrice`, vues de calcul
`portee_effectif` et `nourrice_effectif`. Les références d'historique sont protégées
contre la suppression. Voir la notice utilisateur pour la saisie.


Ce fichier fusionne les anciens audits, listings et listes de modifications.
Les informations devenues obsolètes (anciens comptages de routes, tâches déjà
livrées et doublons) ont été retirées. L'historique destiné aux utilisateurs
est affiché dans l'application sur `/correctifs`; l'historique technique
complet reste disponible dans les commits et demandes de fusion GitHub.

---

## 1. Résumé

Le portage Python 1.65 → Rust est fonctionnellement avancé : le socle
(authentification, SQLite, sécurité, sauvegarde/mise à jour réversible), la
reproduction, les transferts/effectifs, l'économie avec imports PDF, la
productivité (GTTT centralisé), la vente directe, l'eau/électricité et la
saisie rapide sont opérationnels. Les 68 routes historiques encore absentes
après l'audit du 16/08/2026 ont été reliées en 2.2.0 et sont couvertes par un
test de parité (`tests/route_parity.rs`). Le chantier de mise en conformité
avec la spécification complète (types d'élevage, réception d'achats, GTE,
modules optionnels, capacités par étape, prévision d'aliment, rappels
sanitaires, fiche de mise-bas A4 — voir §8) a couvert ses 7 phases fin
août 2026 ; la priorité actuelle porte sur la fiabilité des effectifs
(anciens inventaires incohérents) et les demandes en attente du §3.

---

## 2. Fonctions livrées

- Saisie rapide (bouton « + ») : ELD, poids, NEC, IA, écho, chaleur, mises-bas,
  pertes de porcs et porcelets, sorties, sevrages et mouvements de porcs.
- Centre de pilotage professionnel : huit étapes techniques datées par bande,
  stade actif, progression, prochaine intervention avec délai, effectif et
  site. La feuille de style est versionnée pour empêcher les anciens visuels
  conservés par le navigateur ou Cloudflare.
- Transferts par bande ou unité, avec bâtiment/salle/case de destination.
- Suivi de bande : dates clés, effectif, emplacements, marquage, avance/retard
  sur le départ ou la vente.
- Productivité : GTTT centralisé, objectifs, résultats par bande, données
  techniques d'abattage (TMP, poids, plus-value).
- Économie : coûts par bande, avoirs, valorisations/retenues, variantes
  aliment GR/FE/MI, apports à plusieurs lots.
- Import PDF : aperçu, détection des doublons, transaction globale,
  annulation, journal, import simultané de 1 à 5 PDF (2.2.3).
- Reclassement automatique des avoirs génétiques 1443203 et 1441836, et des
  montants à signe moins final.
- File IA/cochettes après sevrage, affectation groupée à une bande, capacité
  de verraterie paramétrable (31 places par défaut) — 2.2.4.
- Plan sanitaire, pharmacie, liste des porcs traités (hors vaccins).
- Prestataire, RFID, inventaires par case, structure ordonnable,
  eau/électricité, calendrier ICS.
- Sauvegarde SQLite vérifiée, restauration contrôlée, mise à jour Debian
  réversible avec rollback automatique.
- Vente directe : classement filtrable des produits vendus par quantité, kg,
  chiffre d'affaires, prix moyen ou nombre de commandes ; fermeture immédiate
  du formulaire public avec bandeau de fin de vente.
- Tri croissant/décroissant sur les tableaux (classe `.rust-sortable`).

*(Détail des versions : voir la page `/correctifs` de l'application.)*

---

## 3. Reste à faire — priorités

### Fiabilité des données
- [x] Fiabiliser les effectifs réels (anciens inventaires incohérents,
      mortalités avec un stade erroné) — première étape : détection, sans
      correction automatique (l'éleveur reste seul juge de la correction).
      Quatre contrôles ajoutés à l'écran existant `/etat-donnees` (même
      principe que ses contrôles actuels : une valeur à zéro signifie que le
      contrôle est conforme), plutôt qu'un nouvel écran séparé :
      « Cases avec effectif calculé négatif » (mortalités/sorties
      supérieures aux entrées connues depuis le dernier inventaire),
      « Cases dépassant leur capacité déclarée », « Déclarations de
      mortalité sans stade renseigné » et « Porcs charcutiers sans bande
      d'origine ». Vérifié par un test dédié (`etat_donnees_detecte_les_incoherences_deffectif`
      dans `tests/schema.rs`) qui fabrique les quatre situations dans une
      base SQLite en mémoire et vérifie que chaque contrôle les détecte.
- [x] Corriger l'affectation du stade lors des déclarations de mortalité et
      détecter les effectifs incohérents. Les deux points de saisie
      (`/declaration` et la saisie rapide « perte ») laissaient l'utilisateur
      choisir un stade dans une liste fixe indépendante de la case
      sélectionnée, et seul `/declaration` vérifiait l'effectif présent dans
      la case avant d'enregistrer une perte. Le stade est désormais déduit
      automatiquement du type de la salle de la case choisie (mêmes motifs
      que les capacités par étape du §8/Phase 5 — `stade_pour_type_salle`,
      fonction pure testée unitairement), et la saisie rapide applique le
      même contrôle d'effectif insuffisant que `/declaration`. La saisie
      manuelle de « stade » reste utilisée telle quelle quand aucune case
      n'est renseignée (perte sous la mère, cas « Autre »).
- [x] Rattacher les porcs charcutiers à leur bande d'origine (au lieu d'un
      total générique d'engraissement). La donnée existait déjà par bande
      (`total_band_pigs`, utilisé sur la fiche bande), mais l'écran
      Prestataire/Engraissement (`/engraissement`) n'affichait que le
      formulaire de déclaration et le journal de mortalité, sans effectif par
      bande : un prestataire suivant plusieurs bandes ne pouvait pas savoir
      combien de porcs étaient réellement présents pour chacune. Ajout d'un
      tableau « Porcs présents par bande » en tête de cet écran, avec le même
      calcul que la fiche bande (limité aux bandes actives confiées à
      l'engraisseur pour ce rôle, comme le reste de l'écran).

### Conduite d'élevage et productivité
- [x] Produire une GTE complète en complément de la GTTT — déjà livré par le
      chantier §8/Phase 3 (écran `/gte`) ; entrée laissée par erreur dans
      cette liste lors de la clôture du chantier §8, corrigée ici.
- [x] Afficher les ELD dans les fiches truies **sans** garder la moyenne ELD
      par bande. L'historique ELD par truie était déjà affiché sur la fiche
      truie (`/truie/{id}`, section « Mesures ELD, poids et état
      corporel ») ; en revanche `eld_bandes` (moyenne ELD par bande) était
      calculé à chaque chargement de `/productivite` sans être utilisé par
      aucun modèle — code mort supprimé (la moyenne globale `eld_resume`,
      elle, reste affichée et est conservée).
- [x] Afficher TMP et données techniques d'abattage par bande — déjà livré :
      tableau « technique » de `/productivite` (TMP, muscle, plus-value, prix
      net, montant net par bande). Entrée laissée par erreur dans cette
      liste, corrigée ici.
- [x] Afficher stade, emplacement actuel et effectif réel des porcs.
      L'effectif réel était déjà affiché (`suivi_porcs.presents`), mais la
      fiche bande n'avait qu'un journal brut des mouvements (« Derniers
      emplacements enregistrés »), sans vue consolidée. Nouvelle section
      « Emplacement actuel » : une ligne par case où la bande a été
      affectée, avec le stade déduit (même logique que
      `stade_pour_type_salle`, §3 précédent) et l'effectif réellement
      présent dans la case aujourd'hui (`case_pig_count`, temps réel).
      Limite assumée et documentée dans le code : cet effectif compte tous
      les porcs de la case, pas seulement ceux de cette bande, si plusieurs
      bandes y ont été mêlées (le schéma ne trace pas la bande porc par
      porc au-delà du premier mouvement).
- [x] Finaliser la fiche de mise-bas au format A4 définitif — déjà livré par
      le chantier §8/Phase 7 (`/bande/{id}/fiche-mise-bas`). Entrée laissée
      par erreur dans cette liste, corrigée ici.
- [x] Permettre d'ordonner librement les salles dans l'implantation — déjà
      livré : boutons Monter/Descendre sur `/structure`
      (`structure_salle_ordre`, échange transactionnel de l'ordre avec la
      salle voisine du même site). Entrée laissée par erreur dans cette
      liste, corrigée ici.

### Sanitaire
- [x] Rappels sanitaires calculés séparément pour truies, cochettes,
      porcelets et verrats (avec historique). Truies/cochettes/porcelets
      étaient déjà couverts (§8/Phase 6, colonne « Catégorie » = `cible` sur
      `/sanitaire`) ; les verrats restaient un trou documenté (« un verrat
      n'appartient pas à une bande », `acterealise.bande_id` étant
      `NOT NULL`). Fermé par une table additive `acterealiseverrat`
      (mêmes colonnes qu'`acterealise`, sans toucher à sa contrainte
      existante — recréer la table pour l'assouplir aurait été un risque
      inutile sur une base de production) et une route dédiée
      `/sanitaire/fait-verrat`. L'historique « Actes réalisés » réunit
      maintenant bande et verrat par `UNION ALL`. *Limite assumée,
      inchangée :* pas d'échéance calculée pour les rappels verrats (aucune
      date de référence par verrat en base, comme documenté en Phase 6 pour
      les références autres que mise-bas) — seul l'historique était visé ici.
      Un vrai bug SQLite trouvé en écrivant le test dédié
      (`historique_sanitaire_reunit_bandes_et_verrats` dans
      `tests/schema.rs`) : sans alias explicite sur `id`/`date_realise`,
      SQLite refuse l'`ORDER BY` d'un `UNION ALL` (« ORDER BY term does not
      match any column in the result set ») — corrigé avant d'atteindre la
      production.

### Aliment et stock
- [x] Prévision de consommation et de commande d'aliment par bande avant
      rechargement. `/aliment-previsions` calculait déjà « jours avant
      rupture » par silo (§8/Phase 6) mais ne suggérait aucune quantité à
      commander, et n'avait aucune vue par bande. Ajout de deux fonctions
      pures testées (`quantite_a_commander` : tonnage pour ramener le silo à
      sa capacité déclarée ; `commande_urgente` : compare les jours avant
      rupture à un délai de livraison réglable,
      `aliment_delai_commande_jours`, 5 jours par défaut) affichées en badge
      sur chaque silo, et d'une section « Consommation aliment par bande
      (90 derniers jours) » à partir des livraisons déjà rattachées à une
      bande. *Limite assumée :* pas de projection prédictive par bande
      (poids cible/effectif restant varient trop pour un chiffre fiable
      sans intervention de l'éleveur) — visibilité historique par bande,
      volontairement pas une estimation inventée.
- [x] Importer les consommations des machines à soupe. Livré une fois le
      vrai format obtenu (machine Asserva, éleveur ORY EMMANUEL, 5 fichiers
      réels fournis). Trois formats CSV coexistent sur ces machines :
      `Histo_dis` (routage formule → vannes, sans quantité), `Histo_Modif`
      (journal de réglages par vanne) et `Histo_fab` (fabrication —
      quantité consigne/reçue d'eau et de chaque produit nommé par gâchée).
      Seul `Histo_fab` porte de vraies quantités consommées ; c'est le seul
      traité (`src/machine_soupe.rs`). Un cinquième fichier fourni (`Sv_7`,
      sans extension) s'est révélé être un instantané interne au format
      binaire propriétaire (chaînes préfixées par leur longueur, confirmé
      par une lecture hexadécimale) — pas un CSV, délibérément pas géré.
      Deux points techniques réels rencontrés en analysant le vrai fichier :
      encodage Windows-1252/Latin-1 (accents sur un octet, ex. 0xE9 pour
      « é »), pas UTF-8 — un décodage direct aurait échoué ou tronqué le
      fichier ; et le même bug de siècle sur les dates à 2 chiffres que
      les imports PDF Cooperl (`chrono` ne préfixe pas l'année de 2000).
      Import en deux étapes comme les truies (`importjournal`/`importligne`) :
      aperçu listant les produits réels distincts trouvés (les 32 slots
      placeholder `Produit_N` jamais configurés sont filtrés), puis écran de
      correspondance manuelle produit → silo — nouveau silo créé à la volée
      si besoin, mais rien n'est deviné automatiquement, comme demandé.
      Nouvelle table additive `consommationsoupe` (garde le nom exact du
      produit machine pour traçabilité). Affichée en complément
      (pas en remplacement) du bilan de matière existant sur
      `/aliment-previsions`, avec un avertissement sur l'unité (litres ou kg
      selon la machine, jamais convertie au hasard) — l'intégrer au calcul
      de « jours avant rupture » est un incrément naturel séparé si le
      besoin se confirme. Testé contre le vrai fichier `Histo_fab01-26.csv`
      (62 gâchées, 2 produits réels identifiés, dates et quantités
      vérifiées une à une) et par 4 tests unitaires dédiés.

### Économie et imports (demandes en attente)
- [x] Permettre deux lots sur une même facture d'apport Cooperl. Le
      découpage par « Bon n° » (`split_lots`/`parse_lot`) gérait déjà
      plusieurs lots en théorie, mais n'avait jamais été exercé par un test
      ni vérifié contre un vrai PDF. Vérifié et corrigé contre neuf vrais
      bordereaux Cooperl (ORY EMMANUEL) fournis par l'éleveur, dont un à deux
      lots (APPORT N° 226081270686, Bon n° 35776 et 35777) : les 134 porcs et
      les deux poids/montants par lot se retrouvent exactement, et le net à
      payer global se répartit correctement au prorata entre les deux
      lignes de vente. Trois vrais bugs trouvés et corrigés au passage :
      1) un bordereau Cooperl généré par « LDPRX 4.54 » a un octet
      `startxref` qui pointe à côté du mot-clé `xref` (décalage constaté de
      31 octets) — lopdf refusait alors le PDF entier
      (`ParseError::InvalidTrailer`, « Le PDF est illisible, chiffré ou
      endommagé ») alors que le contenu était intact ; `extract_pdf_text`
      retente maintenant avec l'offset réel du mot-clé `xref` quand le
      premier chargement échoue (`repair_startxref_offset`) ; 2) sur le
      modèle de bordereau réellement produit par Cooperl (« duplicata »
      comme « bordereau simplifié »), le libellé « ENLEVEMENT DU » fait
      partie du fond de page (image), pas du texte extrait : la date
      d'enlèvement se retrouvait donc être systématiquement la date de
      facturation (qui suit « LE », elle bien présente comme texte) — la
      date d'enlèvement, elle, apparaît seule sur sa ligne juste avant le
      profil d'élevage (NAISSEUR/ENGRAISSEUR...) ; ajout d'un repli qui la
      capture avant de retomber sur « LE » ; 3) `iso_date` acceptait une
      année à deux chiffres via `%d/%m/%Y` (chrono n'exige pas 4 chiffres à
      la lecture) sans lui ajouter 2000 : chaque date d'un vrai bordereau
      Cooperl (toujours en JJ/MM/AA) était importée avec l'année 0026 au
      lieu de 2026. Les trois corrections sont couvertes par des tests
      dédiés (`repare_un_startxref_decale_comme_le_produit_ldprx`,
      `apport_repartit_deux_bons_sur_la_meme_facture`,
      `apport_prend_la_date_denlevement_pas_la_date_de_facturation`,
      `date_a_deux_chiffres_prend_le_21e_siecle`) construits à partir des
      vrais chiffres et de la vraie mise en forme observée.
- [x] Facture génétique : bien différencier facture et avoir (même
      fournisseur, même modèle de document). En creusant la demande contre
      onze vrais bordereaux Cooperl « ANIMAUX REPRODUCTEURS » fournis par
      l'éleveur (7 factures distinctes + 2 avoirs), constat plus grave que
      prévu : **l'import génétique ne fonctionnait sur aucun vrai
      document**, avoir ou facture — `parse_genetique` cherchait les
      libellés « FACTURE N° », « NET A PAYER » et « BASE H.T. », qui
      n'existent nulle part dans le texte extrait de ce modèle (les libellés
      appartiennent au fond de page image, seules les valeurs sont du vrai
      texte, comme découvert sur les bordereaux d'apport). Résultat vérifié
      avant correction : erreur « aucun montant fiable n'a été trouvé » sur
      la facture réelle la plus simple. Un ancien correctif reclassait à la
      main deux avoirs par leur référence (« 1443203 », « 1441836 ») plutôt
      que de corriger l'extraction — mais cette référence n'était elle-même
      jamais extraite (même défaut de libellé absent), rendant ce correctif
      mort depuis son écriture. Réécrit intégralement sans dépendre d'aucun
      libellé : la référence du document se lit après « Semaine N° » (seul
      repère fiable, ex. « 14.41649 » → 1441649) ; la base H.T. et le total
      T.T.C. se lisent sur l'unique ligne de synthèse TVA sans en-tête
      (« 10400,65   2 5,5%   572,04   10972,69 ») ; le signe (facture ou
      avoir) vient directement du tiret final déjà présent sur chaque nombre
      côté avoir, plus robuste que chercher le mot « AVOIR » — dont
      l'affichage en bannière a ses lettres espacées par le générateur
      (« **** A V O I R **** ») et ne correspond donc jamais à un simple
      mot ; la référence de la facture créditée (« AVOIR SUR FACTURE NO
      1441649 »), elle en toutes lettres, est récupérée et ajoutée au
      libellé (« AVOIR — 28 cochettes (sur facture 1441649) ») pour que
      facture et avoir restent visuellement et durablement reliés sans
      migration de schéma. Bug additionnel corrigé au passage : le comptage
      des animaux (`^([0-9]+)\s+COCHETTE...`) ne reconnaissait aucune ligne
      d'un avoir, où la quantité est elle-même suivie d'un tiret
      (« 27-   COCHETTE... ») — un avoir donnait donc 0 animal quelle que
      soit sa taille réelle. Vérifié contre les onze bordereaux réels (tous
      s'importent désormais, factures et avoirs) ; deux tests dédiés
      (`genetique_reconnait_une_vraie_facture`,
      `genetique_reconnait_un_vrai_avoir_et_le_relie_a_sa_facture`)
      remplacent l'ancien test qui vérifiait le mécanisme mort plutôt que le
      comportement réel.
- [x] Fusionner les variantes de « Charte Qualité Régionale » et vérifier les
      doublons similaires sur toutes les pages. Vrai bug trouvé en écrivant
      le test : `canonical_label` comparait par sous-chaîne sur un texte
      seulement mis en majuscules, donc « CHARTE QUALITE REGI » (sans
      accent) et « CHARTE QUALITÉ RÉGIONALE » (avec accents) ne
      fusionnaient pas — deux lignes distinctes au lieu d'une seule cumulée
      sur *toutes* les pages qui utilisent cette fonction partagée (imports
      PDF Cooperl/Uniporc et saisies manuelles). Corrigé par
      `strip_french_accents`, appliquée avant la comparaison. Vérifié par
      `economic_import::tests::les_10_libelles_de_plus_value_demandes_sont_reconnus`.
- [x] Reconnaître les libellés de plus-value suivants :
      `PARTICIPATION P.S.A. 0J`, `+ VALUE R.S.E.`,
      `PRIME SOLIDARITE JEUNE 5 CT`, `+ VALUE QUALIVIANDE PBE`,
      `COMPLEMENT COCHON DU DIMANC`, `+ VALUE CHARTE QUALITE REGI`,
      `+ VALUE COOPERL LPF`, `+ VALUE PORC SANS ANTIBIOTI`,
      `+ VALUE QUEUE LONGUE (RSE)`, `PARTICIPATION COUT RFID`. Déjà
      reconnus par les entrées existantes de `canonical_label` (vérifié un
      par un, pas supposé) ; ajout du test dédié ci-dessus pour éviter une
      régression silencieuse si `mappings` est modifié plus tard.
- [x] Intégrer le tableau « Cahiers des charges — valorisations » dans
      Économie et retirer sa page séparée. La page `/cahiers` n'apparaissait
      déjà plus dans la navigation (`base.html`) mais restait accessible et
      autonome par URL directe. Son contenu (paramètres des cahiers +
      valorisations réelles importées) est désormais une section de
      `/economique#cahiers` ; `/cahiers` et les actions
      `/cahiers/{id}/maj`/`supprimer` redirigent vers cette ancre pour tout
      favori existant. `templates/cahiers.html` supprimé.
- [x] Supprimer l'estimation prévisionnelle (obsolète) — déjà fait : aucune
      trace de cette fonctionnalité dans le code actuel (recherché sans
      résultat). Entrée laissée par erreur dans cette liste, corrigée ici.
- [x] Regrouper « Lier automatiquement » avec l'import Cooperl et renommer
      l'action « Importer des documents PDF ». Déjà regroupé en une seule
      action (aucun bouton « Lier automatiquement » séparé dans aucun
      template — seules des routes historiques `/economique/auto-lier` et
      `/economique/rattacher-auto` restent enregistrées sans point d'entrée
      dans l'UI, gardées pour la parité des routes Python) ; bouton renommé
      « Importer des documents PDF » (au lieu de « Analyser et lier
      automatiquement ») pour correspondre exactement à la demande.
- [x] Ajouter un modèle d'import des factures génétiques, téléchargeable.
      Traité en 2.2.52 avec le pipeline complet qui manquait — un modèle CSV
      seul, sans point d'entrée pour le consommer, aurait laissé croire à un
      import possible. `/economique/genetique/modele.csv` (en-tête construit
      à partir de la même liste de colonnes que l'analyse, donc impossible à
      désynchroniser), aperçu ligne à ligne sans écriture
      (`/economique/genetique/import`) puis application transactionnelle
      (`.../confirmer`), avec les mêmes garde-fous que les imports existants :
      empreinte SHA-256 du fichier, doublon interne, facture déjà en base,
      contrôle rejoué au moment d'écrire, affectation automatique par bande
      ensuite. Une bande inconnue n'invalide pas la ligne : la facture entre
      non affectée, comme pour un import PDF. Six tests dans
      `src/routes/genetique_import.rs`.

### Présentation et navigation
- [x] Retirer l'ancien texte générique de mise à jour (« remplacer le dossier
      de l'application ») — déjà fait : `templates/maj.html` ne contient
      plus que le texte spécifique au service Debian (dépôt d'archive
      contrôlée, script `mettre-a-jour-debian13.sh`). Recherché sans
      résultat, entrée laissée par erreur dans cette liste.
- [x] Continuer à simplifier la navigation et regrouper les pages proches
      dans des sous-menus (voir structure proposée en §5) — déjà largement
      fait : le menu (`templates/base.html`) est organisé en cinq groupes
      déroulants (Reproduction, Élevage, Pilotage, Aide, Administration)
      plutôt qu'une liste plate, avec un regroupement proche de celui
      suggéré en §5 (ex. Administration ≈ « Paramètres (utilisateurs,
      archives, journal) »). Pas de nouveau regroupement forcé pour
      coller mot à mot à la structure de référence : les pages sont déjà
      correctement classées et un remaniement cosmétique sans besoin
      identifié aurait été du bikeshedding.
- [x] Corriger l'affichage de `/* Capacity badge styles */` en toutes
      lettres sur chaque page (dont la page de connexion). Vrai bug trouvé
      dans `templates/base.html` : ce commentaire CSS était placé entre
      deux balises `<style>`, donc en dehors de tout bloc de style — le
      navigateur l'affichait comme texte brut au lieu de l'interpréter
      comme un commentaire. Déplacé à l'intérieur du bloc `<style>`
      suivant, avec lequel il va (styles des badges de capacité). Vérifié
      qu'aucune autre fuite du même genre n'existe dans les templates.
- [x] Saisie rapide : renommer « Perte porcelet » en « Perte porc » sur le
      bouton d'accès (`templates/base.html`) — changement de libellé
      uniquement, aucun changement de comportement demandé.
- [x] Page `/bandes` : édition « à la volée » de mise-bas, n° de marquage
      et site directement dans le tableau, sans ouvrir la fiche bande
      complète. Chaque ligne a son propre petit formulaire (associé via
      l'attribut HTML `form=` plutôt que des `<form>` imbriqués, invalides
      en HTML) et un bouton « Enregistrer » dédié. Stade et effectif
      restent affichés mais non éditables ici : ce sont des valeurs
      calculées (planning + truies actives), pas des colonnes de `bande` —
      les rendre éditables casserait la cohérence avec la fiche bande, qui
      les recalcule toujours à partir des mêmes données sous-jacentes.
- [x] Eau/électricité : redistribuer la consommation aux bandes selon leur
      présence, avec un tarif au relevé (daté). Le rattachement des bandes
      présentes à un relevé existait déjà (`bandes` sur `releve_compteur`,
      calculé par `present_bands` à la saisie) mais ne servait qu'à
      l'affichage, jamais à répartir un coût. Ajout d'un champ
      `prix_unitaire` par relevé (saisie manuelle et import CSV en masse) ;
      le coût de chaque période (conso × prix du relevé qui la clôture) se
      répartit à parts égales entre les bandes présentes sur cette
      période — à parts égales et non au prorata d'un effectif, car l'eau/
      l'électricité d'un site (lavage, ventilation, chauffage communs) ne
      dépend pas linéairement du nombre de têtes par bande, contrairement à
      l'aliment qui se répartit déjà naturellement par les livraisons
      rattachées à une bande. Nouvelle section « Coût redistribué aux
      bandes » sur `/energie`, cumulée par bande et par type de compteur.
      Deux fonctions pures testées (`cout_consommation`,
      `repartir_cout_par_bande`).
- [x] Mode démo : générateur rejouable produisant plus de 850 truies
      actives, engraissement mixte sur place/extérieur et 5 ans
      d'historique. Nouveau module `src/demo.rs`, appelé par le même bouton
      bascule qu'avant (`/parametres#demo`) — remplace le geste symbolique
      précédent (une bande, une truie, un événement) sans changer son
      fonctionnement pour l'éleveur : toujours un seul bouton, toujours
      retiré proprement via `demoobjet`. Choix pour rester exécutable en
      une seule transaction (~10 s en pratique, vérifié par un test bout en
      bout sur une vraie base SQLite) sans générer des dizaines de milliers
      de lignes inutiles : seules les ~7 bandes les plus récentes (actives,
      un peu plus d'un cycle de reproduction) ont de vraies truies et des
      porcs charcutiers individuels (7×125 = 875 truies actives) ; les
      bandes plus anciennes, jusqu'à 5 ans en arrière (~86 bandes au
      total), n'ont qu'une ligne `bande` avec ses agrégats de production
      (`cs_truies_mb`, `cs_nt_portee`, etc.) déjà remplis — exactement ce
      que lisent les écrans GTTT/productivité historiques, vérifié dans le
      code existant avant d'écrire le générateur plutôt que supposé.
      Engraissement : un tiers des bandes actives est confié à un
      prestataire de démonstration (`bande.engraisseur_id`, même mécanisme
      que les vrais prestataires), le reste reste sur place ; les porcs
      charcutiers portent une `destination` cohérente avec leur bande.
      Nouvelles tables tracées dans `demoobjet` (`site`, `utilisateur`,
      `porccharcutier`) en plus des trois existantes ; suppression
      symétrique mise à jour dans le bon ordre (enfants avant parents, pour
      respecter les clés étrangères). Un vrai piège de compilation trouvé
      et corrigé : `rand::thread_rng()` n'est pas `Send` et ne peut donc
      pas être conservé à travers les nombreux `.await` d'un handler axum —
      remplacé par `StdRng::from_entropy()`. Vérifié contre une vraie base
      SQLite (pas seulement en théorie) : volumes, répartition sur/hors
      site, traçabilité complète dans `demoobjet`, et suppression totale
      sans violation de contrainte.

### Avec service externe (nécessite configuration explicite de l'éleveur)
- [ ] Sauvegardes automatiques vers NAS, stockage en ligne ou messagerie.
- [ ] Synchronisation/export des échéances vers Google Calendar et Outlook.

---

## 4. Feuille de route (versions futures)

| Version | Contenu prévu |
|---|---|
| 2.3.0 | Reproduction, cycles, GTTT/GTE complets |
| 2.4.0 | Effectifs, stock, transferts et bâtiments |
| 2.5.0 | Économie, abattoir et vente directe |
| 2.6.0 | Administration, sauvegarde, mise à jour, finition des écrans |
| Hors calendrier | OCR des scans/photos, imports Excel spécialisés, imports automatiques de référentiels techniques externes, synchronisation directe Gmail/Outlook et stockage cloud (identifiants propres à chaque fournisseur), installateur graphique Windows |

---

## 5. Navigation cible (référence)

Structure proposée pour réduire le nombre de pages visibles (fonctions
secondaires dans des sous-menus) :

Accueil · Productivité · Planning · Bandes · Truies (IA/cochettes, réformes,
attente et réaffectation) · Charcutiers · Comparaison · Plan sanitaire ·
Stocks · Économie · Implantation · Entretien · Paramètres (utilisateurs,
archives, journal) · Contact.

---

## 6. État du portage Python 1.65 → Rust

- La base Python 1.65 comptait 215 routes HTTP pour environ 8 200 lignes de
  code dans son fichier principal. Le portage est une réécriture du moteur,
  pas une conversion automatique.
- Les 68 chemins historiques restés absents après l'audit du 16/08/2026 ont
  été reliés en 2.2.0 et sont couverts par `tests/route_parity.rs`. Le garde-
  fou `501 Not Implemented` ne concerne plus que les URL réellement
  inconnues.
- Dernière validation sur la sauvegarde du 16 août 2026 : intégrité SQLite
  conforme, aucune clé étrangère cassée, 51 tables comparées à la base
  réelle, requêtes principales exécutées sur une copie, aucune donnée
  personnelle incluse dans l'archive de contrôle.

### Règle de validation avant tout déploiement

1. Copie horodatée de la base réelle.
2. Migration appliquée uniquement sur la copie.
3. `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
4. Tests HTTP des routes concernées avec les quatre rôles.
5. Vérification des totaux avant/après import.
6. Sauvegarde puis déploiement atomique avec rollback possible.

La base de production `/var/lib/eo-suivi/elevage.db` ne doit jamais être
utilisée pour développer ou tester une migration.

---

## 7. Déploiement et mise à jour du serveur (Debian 13)

Chemins de production (ne changent pas) :
- sources : `/opt/eo-suivi-rust-src`
- application : `/opt/eo-suivi-rust`
- base : `/var/lib/eo-suivi/elevage.db`
- sauvegardes : `/var/backups/eo-suivi-rust`
- service : `eo-suivi-rust`

La base ne doit jamais être copiée dans le dépôt source. Les versions Python
et Rust ne doivent jamais écrire simultanément dans le même fichier SQLite.

**Mise à jour recommandée** (en `root`) :

```bash
cd /opt/eo-suivi-rust-src
GIT_SSH_COMMAND='ssh -i /root/.ssh/eo-suivi-rust-github -o IdentitiesOnly=yes' \
git pull --ff-only origin main
./scripts/mettre-a-jour-debian13.sh
```

Déploiements suivants, en une seule commande :

```bash
/opt/eo-suivi-rust-src/scripts/mettre-a-jour-debian13.sh
```

Le script :
1. empêche deux mises à jour simultanées (verrou système) ;
2. refuse les modifications locales suivies par Git ;
3. récupère `main` uniquement en avance rapide ;
4. exécute `cargo check`, Clippy, les tests et la compilation release sans
   interrompre le service ;
5. crée une sauvegarde SQLite vérifiée par `PRAGMA quick_check` ;
6. conserve l'ancien binaire et l'ancienne unité systemd ;
7. remplace le binaire atomiquement, redémarre le service et contrôle la page
   de connexion ainsi que le numéro de version ;
8. restaure automatiquement l'ancienne version si l'installation ou le
   contrôle HTTP échoue.

Un échec de compilation laisse l'application en service. Un échec après
l'arrêt déclenche le retour arrière ; la sauvegarde de la base est conservée.

**Vérification manuelle :**

```bash
systemctl status eo-suivi-rust --no-pager -l
journalctl -u eo-suivi-rust -n 80 --no-pager -l -o cat
curl -fsS http://127.0.0.1:8080/login | grep -F "Version Rust 2.2.75"
```

Puis ouvrir `https://rust-elevage.basse-chevrie.ovh`.

---

## 8. Chantier — mise en conformité avec la spécification « naisseur-engraisseur évolutif »

Suivi de l'audit du 19/08/2026 comparant l'application à la spécification
complète (petit élevage familial → structure multi-sites, principe « la
complexité s'active, elle ne s'impose pas »). Chaque phase est une branche et
une PR séparée, testée (`cargo build`/`clippy`/`test`) avant fusion — pas de
gros commit unique sur une application en production.

- [x] **Phase 1 — Type d'élevage (§0bis).** Réglage `parametre.type_elevage`
      (5 profils : naisseur-engraisseur, naisseur, post-sevreur seul,
      post-sevreur-engraisseur, engraisseur seul), écran dans Paramètres,
      propagé dans la session (`SessionData::type_elevage`, rafraîchi en
      direct pour les sessions déjà ouvertes) et utilisé pour masquer les
      menus Reproduction/Charcutiers-Prestataire selon le profil actif.
      Prépare les phases suivantes (§1bis, §7 GTE adaptée).
- [x] **Phase 2 — Réception d'animaux achetés (§1bis).** Table
      `receptionachat` (date, fournisseur, n° bon de livraison, lot d'origine
      fournisseur conservé comme traçabilité, effectif, poids moyen/total,
      prix), écran `/reception` avec formulaire + historique, affectation
      directe à une salle/case (insère aussi un transfert), alerte quarantaine
      à l'arrivée (bannière tant que `quarantaine_jusqu` n'est pas dépassé).
      La réception alimente le registre de mouvements existant
      (`mouvementstock`) au lieu d'une valeur figée, et sa suppression annule
      proprement le mouvement et le transfert associés (pas d'effectif
      fantôme). Visible uniquement pour les profils qui reçoivent des achats
      (`session.recoit_achats()`).
- [x] **Phase 3 — GTE complète (§7), adaptée au type d'élevage.** Nouvel
      écran `/gte` : indice de consommation (IC, kg aliment/kg produit) par
      lot, coût alimentaire par porc produit, marge sur coût alimentaire
      (MSA), marge brute par truie et taux de renouvellement du cheptel (12
      mois glissants). Les colonnes et la section « Renouvellement » propres
      aux truies ne s'affichent que pour les profils qui en conduisent
      (`session.a_truies()`, Phase 1) — vérifié en conditions réelles en
      basculant un lot de naisseur-engraisseur à post-sevreur. 5 fonctions de
      calcul pures et testées unitairement (indice_consommation,
      cout_alimentaire_par_porc, marge_sur_cout_alimentaire,
      marge_brute_par_truie, taux_renouvellement_pct). *Complété en 2.2.52 :*
      le coût d'achat des animaux entrants (`receptionachat`, Phase 2) est
      désormais imputé au lot comme charge d'entrée. La MSA garde sa
      définition (recettes moins le seul aliment) ; une colonne « Marge après
      achat » la corrige pour les profils acheteurs et sert de base à la marge
      brute par truie. Sans réception d'achat, le coût vaut 0 et rien ne
      change pour un naisseur ou un naisseur-engraisseur. Requête isolée en
      constante `GTE_LOTS_SQL`, rejouée telle quelle par un test sur base
      SQLite en mémoire. *Reste hors périmètre :* l'indice de consommation est
      calculé sur le poids vendu, pas sur le gain de poids depuis l'entrée —
      il reste donc optimiste pour un engraisseur qui achète des animaux déjà
      lourds ; à traiter avec le poids d'entrée par lot.
- [x] **Phase 4 — Modules optionnels (§0, §2, §4).** Cases à cocher
      « Génétique avancée » (décochée par défaut) et « Prestataires externes »
      (cochée par défaut, préserve le comportement des bases existantes) dans
      Paramètres. Catalogue de lignées (`/genetique` : nom, fournisseur,
      index prolificité/croissance/IC, contrat de renouvellement) visible
      uniquement module actif. Lien « Prestataire » masqué quand le module
      est désactivé (Charcutiers/engraissement restent disponibles : seule la
      sous-traitance externe est masquée). Piège vérifié en conditions
      réelles : la page Paramètres a deux formulaires distincts postant vers
      `/parametres/maj` (type d'élevage+modules, et informations générales) —
      soumettre l'un ne doit pas réinitialiser les cases à cocher de l'autre ;
      un garde-fou (`form.contains_key("type_elevage")`) l'empêche, testé par
      soumission successive des deux formulaires. *Complété en 2.2.52 :* une
      truie se rattache à une lignée du catalogue par la colonne additive
      `truie.lignee_id` (`POST /truie/{id}/lignee`, sélecteur visible
      seulement module actif). `truie.race`, texte libre historique, est
      conservé tel quel : aucune donnée réécrite, aucun élevage obligé
      d'utiliser le catalogue. Piège vérifié en conditions réelles :
      `truie.lignee_id` étant une clé étrangère, supprimer une lignée encore
      utilisée renvoyait une erreur technique — la suppression est désormais
      refusée avec le nombre de truies concernées, et `/genetique` affiche
      cette colonne.
- [x] **Phase 5 — Capacités par étape (§0, §3).** Trois nouveaux réglages
      (`capacite_maternite`=60, `capacite_postsevrage`=300,
      `capacite_engraissement`=800, éditables comme `capacite_verraterie`
      existant dans Réglages de conduite) et une carte « Capacités par
      étape » sur le tableau de bord : occupation recalculée à partir des
      truies/porcs réellement présents dans les salles correspondantes
      (`salle.type`), jamais une valeur figée. Limitée aux phases réellement
      actives pour le type d'élevage (verraterie/maternité seulement si
      `a_truies()`, engraissement seulement si `engraisse()`) — vérifié en
      conditions réelles en plaçant une truie en maternité (occupation passe
      à 1/60) puis en basculant vers un profil post-sevreur (seul le
      post-sevrage reste affiché). *Conduite en continu / hors bande : déjà
      fonctionnellement supportée (bande_code nullable, écran « Sans bande
      active » existant) — non repris ici faute de gain identifié au-delà de
      l'existant ; à documenter comme mode nommé si un besoin utilisateur
      concret apparaît.*
- [x] **Phase 6 — Prévision d'aliment et rappels sanitaires par catégorie.**
      *Aliment* : écran `/aliment-previsions`, silos + relevés de niveau
      manuels (même principe que `compteur_energie`/`releve_compteur`
      existant) ; la consommation quotidienne se déduit par bilan de matière
      entre deux relevés et les livraisons reçues entre-temps, avec
      estimation du nombre de jours avant rupture. *Sanitaire* : les colonnes
      `cible`/`reference`/`jour`/`rappel` d'`acteprotocole` existaient déjà en
      base mais n'étaient exploitées par aucun calcul ; une section
      « Rappels » sur `/sanitaire` liste maintenant, par catégorie, les actes
      marqués « rappel » dont l'échéance (mise-bas + décalage en jours) est
      atteinte pour une bande active et pas encore réalisée, avec repère
      visuel « en retard ». Un bug réel a été trouvé et corrigé en testant en
      conditions réelles : `printf('+%d day', jour)` produisait un modificateur
      SQLite invalide (`+-14 day`) pour un décalage négatif (ex. vaccin avant
      mise-bas) ; corrigé en `printf('%+d day', jour)`. *Limite levée depuis :*
      l'historique des rappels verrats, annoncé ici comme non couvert, l'est
      par la table additive `acterealiseverrat` et la route
      `/sanitaire/fait-verrat` (voir §3, Sanitaire). Seule subsiste l'absence
      d'échéance calculée pour ces rappels, faute de date de référence par
      verrat en base.
- [x] **Phase 7 — Fiche de mise-bas au format A4 définitif (§9).** Nouvel
      écran `/bande/{id}/fiche-mise-bas`, mise en page dédiée (`@page { size:
      A4; margin: 15mm; }`) distincte du dump générique `impression.html` :
      en-tête avec nom d'élevage (paramètre `nom_elevage`), code de bande,
      date de mise-bas et site, ligne de totaux (truies, nés totaux/vifs,
      mort-nés, momifiés, sevrés), tableau détaillé par truie (même requête
      que l'export CSV existant, pour rester cohérent entre les deux formats)
      et zone de signature éleveur/vétérinaire pour l'archivage papier du
      registre d'élevage. Lien ajouté sur la fiche bande, à côté de l'export
      CSV existant. Vérifié en conditions réelles avec et sans données.

Chaque case cochée référence sa PR dans l'historique Git de ce dépôt.

#!/usr/bin/env bash
# Installation initiale d'EO_Sevel sur un serveur Debian 13 neuf.
#
# À lancer une seule fois, en root, depuis le dossier des sources. La mise à
# jour ultérieure se fait avec mettre-a-jour-debian13.sh, qui sauvegarde et
# restaure en cas d'échec — ce script-ci ne sait qu'installer du neuf et
# refuse d'écraser une installation existante.
#
#   sudo ./scripts/installer-debian13.sh
set -euo pipefail

SRC_DIR="${EO_SOURCE_DIR:-/opt/eo-sevel-src}"
APP_DIR="${EO_APP_DIR:-/opt/eo-sevel}"
DATA_DIR="${EO_DATA_DIR:-/var/lib/eo-sevel}"
BACKUP_DIR="${EO_BACKUP_DIR:-/var/backups/eo-sevel}"
COMPTE="${EO_USER:-eo-sevel}"
SERVICE="${EO_SERVICE:-eo-sevel}"

if [[ "${EUID}" -ne 0 ]]; then
    echo "Ce script doit être lancé en root (sudo)." >&2
    exit 1
fi

racine=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$racine"

if [[ ! -f Cargo.toml ]]; then
    echo "Lancez ce script depuis les sources d'EO_Sevel." >&2
    exit 1
fi

# Une installation existante se met à jour, elle ne se réinstalle pas : écraser
# ici ferait perdre la base sans sauvegarde.
if [[ -e "$DATA_DIR/elevage.db" ]]; then
    echo "Une base existe déjà dans $DATA_DIR." >&2
    echo "Utilisez scripts/mettre-a-jour-debian13.sh pour mettre à jour." >&2
    exit 1
fi

echo "== Dépendances système =="
apt-get update
apt-get install -y --no-install-recommends ca-certificates curl build-essential pkg-config

if ! command -v cargo >/dev/null 2>&1; then
    echo "Rust n'est pas installé. Installez-le avec https://rustup.rs puis relancez." >&2
    exit 1
fi

echo "== Compilation =="
cargo build --locked --release
for binaire in eo-sevel eo-regul; do
    if [[ ! -f "target/release/$binaire" ]]; then
        echo "Binaire $binaire absent après compilation." >&2
        exit 1
    fi
done

echo "== Compte système et dossiers =="
if ! id -u "$COMPTE" >/dev/null 2>&1; then
    useradd --system --home-dir "$DATA_DIR" --shell /usr/sbin/nologin "$COMPTE"
fi
install -d -o "$COMPTE" -g "$COMPTE" -m 0750 "$APP_DIR" "$DATA_DIR" "$BACKUP_DIR"
install -d -o "$COMPTE" -g "$COMPTE" -m 0750 "$APP_DIR/static"

echo "== Binaires et ressources =="
install -m 0755 -o "$COMPTE" -g "$COMPTE" target/release/eo-sevel "$APP_DIR/eo-sevel"
install -m 0755 -o "$COMPTE" -g "$COMPTE" target/release/eo-regul "$APP_DIR/eo-regul"
cp -a static/. "$APP_DIR/static/"
chown -R "$COMPTE:$COMPTE" "$APP_DIR"

if [[ "$racine" != "$SRC_DIR" ]]; then
    echo "Note : les sources sont dans $racine, alors que la mise à jour"
    echo "automatique attend $SRC_DIR. Déplacez-les ou ajustez EO_SOURCE_DIR"
    echo "dans eo-sevel-update.service."
fi

echo "== Services =="
install -m 0644 eo-sevel.service /etc/systemd/system/eo-sevel.service
install -m 0644 eo-sevel-update.service /etc/systemd/system/eo-sevel-update.service
install -m 0644 eo-sevel-update.path /etc/systemd/system/eo-sevel-update.path
install -m 0644 eo-regul.service /etc/systemd/system/eo-regul.service
systemctl daemon-reload
systemctl enable --now "$SERVICE"
systemctl enable --now eo-sevel-update.path

# La régulation n'est PAS démarrée ici, et ce n'est pas un oubli. Mettre en
# route une ventilation se décide à la main, après avoir renseigné les salles
# et vérifié les adresses de l'automate. Le service est installé en mode
# simulation : même démarré par erreur, il n'écrit rien sur l'automate.
echo
echo "== Contrôle de santé =="
for tentative in $(seq 1 30); do
    if curl -fsS --max-time 2 "http://127.0.0.1:${EO_PORT:-8080}/login" >/dev/null 2>&1; then
        echo "Serveur en ligne."
        break
    fi
    if [[ "$tentative" -eq 30 ]]; then
        echo "Le serveur ne répond pas. Voir : journalctl -u $SERVICE -n 50" >&2
        exit 1
    fi
    sleep 1
done

cat <<'FIN'

Installation terminée.

  Interface     http://<adresse-du-serveur>:8080
  Identifiants  admin / admin — un mot de passe vous sera demandé aussitôt

Régulation de ventilation : le service eo-regul est installé mais volontairement
arrêté, et réglé en mode simulation.

  1. Créez vos salles dans « Structure ».
  2. Dans « Ambiance et ventilation », renseignez pour chaque salle l'adresse de
     l'automate et les identifiants d'entrées/sorties, puis cochez « Mesurer ».
     Ne cochez pas encore « Piloter ».
  3. Vérifiez les gabarits d'accès à l'automate (table parametre, clés
     ambiance_ipx_*) contre le firmware réel de votre IPX800.
  4. Démarrez la régulation en observateur :
         systemctl enable --now eo-regul
     Elle mesure, enregistre et alerte, sans rien écrire sur l'automate.
  5. Comparez avec la régulation en place sur une bande complète. Ensuite
     seulement, retirez EO_REGUL_SIMULATION de l'unité et cochez « Piloter »,
     une salle à la fois.

Avant de piloter une maternité : la règle de repli doit exister dans l'automate
(surveillance du battement de cœur, bascule sur un pourcentage de sécurité).
Sans elle, un redémarrage du serveur laisse les salles sans commande.
FIN

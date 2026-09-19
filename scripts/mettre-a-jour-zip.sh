#!/usr/bin/env bash
set -Eeuo pipefail

export PATH="/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"

# Le serveur qui compile est le même petit conteneur qui fait tourner
# l'élevage : une compilation à pleine parallélisation peut demander plus de
# RAM qu'il n'en a (observé : swap saturé à 100 %, conteneur redémarré de
# force par l'hyperviseur). Un seul job à la fois est plus lent mais tient
# dans la mémoire disponible.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-1}"

SRC_DIR="${EO_SOURCE_DIR:-/opt/eo-sevel-src}"
APP_DIR="${EO_APP_DIR:-/opt/eo-sevel}"
DB_FILE="${EO_DATABASE:-/var/lib/eo-sevel/elevage.db}"
BACKUP_DIR="${EO_BACKUP_DIR:-/var/backups/eo-sevel}"
SERVICE="${EO_SERVICE:-eo-sevel}"
LOGIN_URL="${EO_LOGIN_URL:-http://127.0.0.1:8080/login}"
STYLE_URL="${EO_STYLE_URL:-http://127.0.0.1:8080/static/style.css}"
STATUS_FILE="${EO_UPDATE_STATUS:-$(dirname "$DB_FILE")/mise-a-jour-statut}"
MAJ_DIR="${EO_UPDATE_DIR:-/var/lib/eo-sevel/maj}"
ZIP_FILE="${1:-${EO_UPDATE_ZIP:-$MAJ_DIR/eo-sevel-nouvelle-version.zip}}"
ARCHIVE_DIR="${EO_UPDATE_ARCHIVE:-$MAJ_DIR/traitees}"

status_update() {
    local state="$1" percent="$2" release="$3" message="$4"
    local status_tmp="${STATUS_FILE}.tmp"
    mkdir -p "$(dirname "$STATUS_FILE")" || true
    printf '%s\n%s\n%s\n%s\n' "$state" "$percent" "$release" "$message" >"$status_tmp" 2>/dev/null || true
    mv -f "$status_tmp" "$STATUS_FILE" 2>/dev/null || true
    chown eo-sevel:eo-sevel "$STATUS_FILE" 2>/dev/null || true
}

die() {
    status_update "erreur" "100" "${version:-}" "$*"
    echo "Erreur : $*" >&2
    exit 1
}

if [[ $(id -u) -ne 0 ]]; then
    die "cette commande doit être lancée avec root."
fi

for command in cargo curl flock install rsync sqlite3 systemctl unzip; do
    command -v "$command" >/dev/null 2>&1 || die "commande manquante : $command"
done

exec 9>/run/lock/maj-eo-sevel.lock
flock -n 9 || die "une mise à jour EO_Sevel est déjà en cours."
status_update "demarrage" "5" "" "Démarrage de la mise à jour."

[[ -f "$ZIP_FILE" ]] || die "archive zip introuvable : $ZIP_FILE"
[[ -f "$DB_FILE" ]] || die "base SQLite introuvable : $DB_FILE"

mkdir -p "$BACKUP_DIR" "$APP_DIR" "$ARCHIVE_DIR"

echo "Extraction de l'archive $ZIP_FILE..."
status_update "telechargement" "12" "" "Extraction de l'archive."
extract_dir=$(mktemp -d)
cleanup_extract() { rm -rf "$extract_dir"; }
trap cleanup_extract EXIT

unzip -q "$ZIP_FILE" -d "$extract_dir" || die "l'archive est invalide ou corrompue."

# L'archive peut contenir soit le contenu du dépôt directement, soit un
# unique dossier racine (ex: eo-sevel/) qui l'enveloppe.
payload_dir="$extract_dir"
if [[ ! -f "$payload_dir/Cargo.toml" ]]; then
    mapfile -t top_entries < <(find "$extract_dir" -mindepth 1 -maxdepth 1)
    if [[ ${#top_entries[@]} -eq 1 && -d "${top_entries[0]}" && -f "${top_entries[0]}/Cargo.toml" ]]; then
        payload_dir="${top_entries[0]}"
    fi
fi
[[ -r "$payload_dir/Cargo.toml" ]] || die "Cargo.toml introuvable dans l'archive $ZIP_FILE"

echo "Mise à jour des sources dans $SRC_DIR..."
rsync -a --delete --exclude target --exclude '.git' "$payload_dir/" "$SRC_DIR/"

trap - EXIT
cleanup_extract

cd "$SRC_DIR"

version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
[[ -n "$version" ]] || die "version Cargo introuvable."
expected_style=$(<static/style.css)

if systemctl is-active --quiet "$SERVICE"; then
    current_page=$(curl -fsS "$LOGIN_URL" 2>/dev/null || true)
    current_style=$(curl -fsS "$STYLE_URL?v=$version" 2>/dev/null || true)
    if grep -Fq "Version Rust $version" <<<"$current_page" \
        && [[ "$current_style" == "$expected_style" ]]; then
        status_update "terminee" "100" "$version" "La version $version est déjà installée."
        echo "EO_Sevel $version et son interface sont déjà installés."
        mkdir -p "$ARCHIVE_DIR"
        mv -f "$ZIP_FILE" "$ARCHIVE_DIR/eo-sevel-${version}_$(date +%Y%m%d_%H%M%S)_deja-installee.zip" 2>/dev/null || true
        exit 0
    fi
fi

echo "Contrôles et compilation de la version $version..."
status_update "controles" "25" "$version" "Contrôles du code et des dépendances."
cargo check
cargo clippy --all-targets -- -D warnings
cargo test
status_update "compilation" "50" "$version" "Compilation de la version $version."
cargo build --release

stamp=$(date +%Y%m%d_%H%M%S)
db_backup="$BACKUP_DIR/elevage_avant_${version}_${stamp}.db"
binary_backup="$BACKUP_DIR/eo-sevel_${stamp}"
unit_backup="$BACKUP_DIR/eo-sevel.service_${stamp}"
static_backup="$BACKUP_DIR/static_${stamp}"
new_binary="$APP_DIR/.eo-sevel.new-${stamp}"

echo "Sauvegarde contrôlée de la base : $db_backup"
status_update "sauvegarde" "70" "$version" "Sauvegarde et contrôle de la base SQLite."
sqlite3 "$DB_FILE" ".backup '$db_backup'"
[[ $(sqlite3 "$db_backup" "PRAGMA quick_check;") == "ok" ]] || die "la sauvegarde SQLite est invalide."

binary_saved=0
unit_saved=0
static_saved=0
if [[ -f "$APP_DIR/eo-sevel" ]]; then
    cp -a "$APP_DIR/eo-sevel" "$binary_backup"
    binary_saved=1
fi
if [[ -f /etc/systemd/system/eo-sevel.service ]]; then
    cp -a /etc/systemd/system/eo-sevel.service "$unit_backup"
    unit_saved=1
fi
if [[ -d "$APP_DIR/static" ]]; then
    cp -a "$APP_DIR/static" "$static_backup"
    static_saved=1
fi

# Préparer le nouveau binaire sur le même système de fichiers permet au mv
# suivant de remplacer l'exécutable de façon atomique.
install -m 0755 target/release/eo-sevel "$new_binary"
chown eo-sevel:eo-sevel "$new_binary"

rollback_required=0
restore_previous_release() {
    status=$?
    trap - EXIT
    status_update "erreur" "100" "${version:-}" "Échec de la mise à jour ; restauration de la version précédente."
    if [[ "$rollback_required" -eq 1 ]]; then
        echo "Échec du déploiement : restauration de la version précédente." >&2
        systemctl stop "$SERVICE" >/dev/null 2>&1 || true
        if [[ "$binary_saved" -eq 1 ]]; then
            install -m 0755 "$binary_backup" "$APP_DIR/eo-sevel"
            chown eo-sevel:eo-sevel "$APP_DIR/eo-sevel"
        fi
        if [[ "$unit_saved" -eq 1 ]]; then
            install -m 0644 "$unit_backup" /etc/systemd/system/eo-sevel.service
        fi
        if [[ "$static_saved" -eq 1 ]]; then
            mkdir -p "$APP_DIR/static"
            cp -a "$static_backup/." "$APP_DIR/static/"
        fi
        systemctl daemon-reload || true
        systemctl start "$SERVICE" || true
        echo "La base sauvegardée reste disponible dans $db_backup" >&2
    fi
    exit "$status"
}
trap restore_previous_release EXIT

rollback_required=1
echo "Installation atomique de la version $version..."
status_update "installation" "82" "$version" "Installation de la nouvelle version et redémarrage."
systemctl stop "$SERVICE"
mv "$new_binary" "$APP_DIR/eo-sevel"

# Démon de régulation. Il est déployé à chaque mise à jour, mais jamais activé
# automatiquement : mettre en route une ventilation doit rester une décision
# prise à la main, jamais l'effet de bord d'une mise à jour.
if [[ -f target/release/eo-regul ]]; then
    if systemctl is-active --quiet eo-regul; then
        systemctl stop eo-regul || true
        regul_etait_actif=1
    else
        regul_etait_actif=0
    fi
    install -m 0755 target/release/eo-regul "$APP_DIR/eo-regul"
    install -m 0644 eo-regul.service /etc/systemd/system/eo-regul.service
    chown eo-sevel:eo-sevel "$APP_DIR/eo-regul"
fi
mkdir -p "$APP_DIR/static"
cp -a static/. "$APP_DIR/static/"
install -m 0644 eo-sevel.service /etc/systemd/system/eo-sevel.service
install -m 0644 eo-sevel-update.service /etc/systemd/system/eo-sevel-update.service
install -m 0644 eo-sevel-update.path /etc/systemd/system/eo-sevel-update.path
chown -R eo-sevel:eo-sevel "$APP_DIR"
systemctl daemon-reload
systemctl enable "$SERVICE" >/dev/null
systemctl enable --now eo-sevel-update.path >/dev/null
systemctl start "$SERVICE"
if [[ "${regul_etait_actif:-0}" -eq 1 ]]; then
    systemctl start eo-regul || true
fi

healthy=0
health_detail="le service ne répond pas encore"
status_update "verification" "94" "$version" "Vérification du redémarrage et de l'interface."
for _ in {1..15}; do
    if ! current_page=$(curl -fsS "$LOGIN_URL" 2>/dev/null); then
        health_detail="la page de connexion ne répond pas"
    elif ! grep -Fq "Version Rust $version" <<<"$current_page"; then
        health_detail="la page de connexion ne présente pas la version $version"
    elif ! current_style=$(curl -fsS "$STYLE_URL?v=$version" 2>/dev/null); then
        health_detail="la feuille de style ne répond pas"
    elif [[ "$current_style" != "$expected_style" ]]; then
        health_detail="la feuille de style servie ne correspond pas au fichier installé"
    else
        healthy=1
        break
    fi
    sleep 1
done
if [[ "$healthy" -ne 1 ]]; then
    systemctl --no-pager --full status "$SERVICE" >&2 || true
    journalctl -u "$SERVICE" -n 40 --no-pager -o cat >&2 || true
    die "le contrôle HTTP a échoué : $health_detail."
fi

rollback_required=0
trap - EXIT
status_update "terminee" "100" "$version" "Mise à jour terminée avec succès."
mkdir -p "$ARCHIVE_DIR"
mv -f "$ZIP_FILE" "$ARCHIVE_DIR/eo-sevel-${version}_${stamp}.zip" 2>/dev/null || true

echo "Mise à jour terminée : EO_Sevel $version"
echo "Sauvegarde SQLite : $db_backup"
systemctl --no-pager --full status "$SERVICE" | sed -n '1,12p'

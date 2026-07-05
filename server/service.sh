#!/bin/bash
# service.sh — установка backup-server как systemd-сервис на VPS.
#
#   /opt/backup-server/
#     service.sh
#     backup-server          ← скопируйте бинарник отдельно (не в bundle клиента)
#     config.toml
#     tls/server.crt, tls/server.key
#
# Usage:
#   cd /opt/backup-server && sudo ./service.sh
#   sudo INSTALL_USER=backup ./service.sh
#   sudo MEMORY_MAX=512M ./service.sh

set -euo pipefail

trap 'echo "Ошибка на строке ${LINENO} (код $?)" >&2' ERR

if [[ "${EUID:-$(id -u)}" -ne 0 ]]; then
    echo "Запустите от root: sudo $0"
    exit 1
fi

INSTALL_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SERVER_PATH="$INSTALL_DIR/backup-server"
CONFIG_PATH="$INSTALL_DIR/config.toml"
TLS_CERT="$INSTALL_DIR/tls/server.crt"
TLS_KEY="$INSTALL_DIR/tls/server.key"
SERVICE_NAME="${SERVICE_NAME:-backup-server}"
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"
START_WAIT_SEC="${START_WAIT_SEC:-20}"

echo "Установка ${SERVICE_NAME}..."
echo "  Папка        : $INSTALL_DIR"

if [[ ! -f "$SERVER_PATH" ]]; then
    echo "Ошибка: не найден backup-server: $SERVER_PATH"
    echo "  Скопируйте musl-бинарник backup-server в эту папку и: chmod +x backup-server"
    exit 1
fi
chmod +x "$SERVER_PATH"

if [[ ! -f "$CONFIG_PATH" ]]; then
    echo "Ошибка: не найден config.toml: $CONFIG_PATH"
    exit 1
fi

if [[ ! -f "$TLS_CERT" || ! -f "$TLS_KEY" ]]; then
    echo "Ошибка: нужны TLS-файлы из bundle клиента:"
    echo "  $TLS_CERT"
    echo "  $TLS_KEY"
    exit 1
fi

# Пользователь сервиса: env → владелец бинарника (если не root) → SUDO_USER → logname
if [[ -z "${INSTALL_USER:-}" ]]; then
    BIN_OWNER="$(stat -c '%U' "$SERVER_PATH" 2>/dev/null || echo root)"
    if [[ "$BIN_OWNER" != "root" ]]; then
        INSTALL_USER="$BIN_OWNER"
    elif [[ -n "${SUDO_USER:-}" ]]; then
        INSTALL_USER="$SUDO_USER"
    else
        INSTALL_USER="$(logname 2>/dev/null || echo root)"
    fi
fi
INSTALL_GROUP="${INSTALL_GROUP:-$INSTALL_USER}"

if [[ "$INSTALL_USER" == "root" ]]; then
    echo "Предупреждение: сервис будет работать от root."
    echo "  Рекомендуется: sudo INSTALL_USER=ваш_user $0"
fi

echo "  Пользователь : $INSTALL_USER"

read_toml_value() {
    local key="$1"
    grep -E "^[[:space:]]*${key}[[:space:]]*=" "$CONFIG_PATH" 2>/dev/null \
        | head -1 \
        | cut -d= -f2- \
        | tr -d ' "' \
        | tr -d "'" \
        || true
}

PUBLIC_IP="$(read_toml_value public_ip)"
PUBLIC_IP="${PUBLIC_IP:-127.0.0.1}"

SERVER_PORT="$(read_toml_value server_port)"
SERVER_PORT="${SERVER_PORT:-8080}"

DEBUG_MODE="$(read_toml_value debug)"
if [[ -z "$DEBUG_MODE" ]]; then
    LOGGER="$(read_toml_value logger)"
    if [[ "$LOGGER" == "full" ]]; then
        DEBUG_MODE=1
    else
        DEBUG_MODE=0
    fi
fi

MEMORY_LINE=""
if [[ -n "${MEMORY_MAX:-}" ]]; then
    MEMORY_LINE="MemoryMax=${MEMORY_MAX}"
    echo "  MemoryMax    : $MEMORY_MAX"
fi

echo "  Права        : chown $INSTALL_USER на $INSTALL_DIR"
mkdir -p "$INSTALL_DIR/data/temp" "$INSTALL_DIR/data/scripts"
chown -R "$INSTALL_USER:$INSTALL_GROUP" "$INSTALL_DIR"
chmod 755 "$INSTALL_DIR"
chmod +x "$SERVER_PATH"
chmod 644 "$CONFIG_PATH"
chmod 644 "$TLS_CERT"
chmod 600 "$TLS_KEY"
chmod 755 "$INSTALL_DIR/service.sh" 2>/dev/null || true

cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Backup Server (Rust)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=$INSTALL_USER
Group=$INSTALL_GROUP
WorkingDirectory=$INSTALL_DIR
ExecStart=$SERVER_PATH
Restart=always
RestartSec=5
KillMode=mixed
SendSIGKILL=yes
TimeoutStopSec=15
StandardOutput=journal
StandardError=journal
SyslogIdentifier=$SERVICE_NAME
RuntimeDirectory=$SERVICE_NAME
RuntimeDirectoryMode=0755
${MEMORY_LINE}

[Install]
WantedBy=multi-user.target
EOF

chmod 644 "$SERVICE_FILE"
echo "  Unit-файл    : $SERVICE_FILE"

systemctl daemon-reload
systemctl enable "$SERVICE_NAME"

if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
    systemctl restart "$SERVICE_NAME"
else
    systemctl start "$SERVICE_NAME"
fi

echo ""
echo "Ожидание запуска (до ${START_WAIT_SEC}s)..."
STARTED=0
for _ in $(seq 1 "$START_WAIT_SEC"); do
    if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
        if journalctl -u "$SERVICE_NAME" -n 30 --no-pager 2>/dev/null \
            | grep -qE "Listening on https://"; then
            STARTED=1
            break
        fi
    fi
    sleep 1
done

echo ""
if [[ "$STARTED" -eq 1 ]]; then
    echo "OK: сервис запущен."
    systemctl status "$SERVICE_NAME" --no-pager -l | head -20 || true
else
    echo "ОШИБКА: сервис не перешёл в рабочее состояние за ${START_WAIT_SEC}s."
    systemctl status "$SERVICE_NAME" --no-pager -l | head -25 || true
    echo ""
    echo "Последние логи:"
    journalctl -u "$SERVICE_NAME" -n 25 --no-pager || true
    exit 1
fi

echo ""
echo "Логи:      journalctl -u $SERVICE_NAME -f"
echo "Статус:    systemctl status $SERVICE_NAME"
if [[ "$DEBUG_MODE" == "1" ]]; then
    echo "Debug:     debug=true — предупреждения при RSS > 100 MB"
fi
echo "WebSocket: wss://${PUBLIC_IP}:${SERVER_PORT}/ws"
echo "Health:    https://${PUBLIC_IP}:${SERVER_PORT}/health"

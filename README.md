# Webhook Relay

Service de relais de webhooks. Les clients de l'API enregistrent des endpoints HTTP, envoient des événements JSON, et le service les livre en `POST` avec une signature HMAC-SHA256. Les échecs sont retentés avec un backoff exponentiel, et chaque tentative reste consultable.

Le développement se fait contre MySQL installé en natif. Le `Dockerfile` sert uniquement au déploiement. Redis n'est pas utilisé : la file de livraison est la table `delivery_attempts`.

## Prérequis

- Rust stable (1.94 ou plus récent)
- MySQL 8, en écoute sur `127.0.0.1:3306`
- [sqlx-cli](https://github.com/launchbadge/sqlx) 0.9, pour créer la base et vérifier les requêtes à la compilation

```bash
cargo install sqlx-cli --no-default-features --features rustls,mysql,mysql-rsa
```

La feature `mysql-rsa` est nécessaire pour une connexion MySQL 8 sans TLS (`caching_sha2_password`).

## Installer MySQL en natif

### macOS (Homebrew)

```bash
brew install mysql
brew services start mysql
mysql -u root -e "ALTER USER 'root'@'localhost' IDENTIFIED BY 'password';"
```

### Linux (Debian / Ubuntu)

```bash
sudo apt-get update
sudo apt-get install -y mysql-server
sudo systemctl enable --now mysql
sudo mysql -e "ALTER USER 'root'@'localhost' IDENTIFIED WITH caching_sha2_password BY 'password'; FLUSH PRIVILEGES;"
```

Sur certaines images, l'utilisateur administrateur initial est `auth_socket`. Dans ce cas :

```bash
sudo mysql
```

```sql
CREATE USER 'webhook'@'127.0.0.1' IDENTIFIED BY 'password';
GRANT ALL PRIVILEGES ON webhook_relay.* TO 'webhook'@'127.0.0.1';
FLUSH PRIVILEGES;
```

Adapte ensuite `DATABASE_URL`.

### Windows

Le service `MySQL80` de MySQL Installer convient. Utilise le compte `root` choisi à l'installation dans `DATABASE_URL`.

## Créer la base et appliquer les migrations

```bash
cp .env.example .env
```

Édite `DATABASE_URL` et `JWT_SECRET` (32 caractères minimum).

```bash
sqlx database create
sqlx migrate run
```

Équivalent SQL :

```sql
CREATE DATABASE webhook_relay CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
```

Les UUID sont générés par l'application et stockés en `CHAR(36)`. Les dates sont des `DATETIME(6)` en UTC, sans fuseau. Le charset des tables est `utf8mb4`.

`cargo run` relance aussi les migrations au démarrage : elles sont idempotentes.

Les requêtes `sqlx::query!` sont vérifiées à la compilation. Avec `DATABASE_URL` défini, SQLx interroge MySQL. Le dépôt contient aussi le cache hors-ligne `.sqlx`, utilisé par l'image Docker (`SQLX_OFFLINE=true`). Après une modification SQL :

```bash
cargo sqlx prepare
```

## Lancer le service

```bash
cargo run
```

L'API écoute sur `BIND_ADDR` (défaut `0.0.0.0:8080`). Un worker Tokio tourne dans le même processus : toutes les secondes il réserve les livraisons échues et les envoie en parallèle, avec une limite de concurrence.

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` a besoin de `DATABASE_URL` et écrit dans cette base. Utilise une base dédiée.

## Flux complet

Les exemples utilisent `curl`. Sous PowerShell, préfère `curl.exe` et `$token` à la place de la substitution bash.

### 1. Créer un compte et récupérer un JWT

```bash
curl -s -X POST http://127.0.0.1:8080/auth/register \
  -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"correct-horse"}'

curl -s -X POST http://127.0.0.1:8080/auth/login \
  -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"correct-horse"}'
```

La réponse de login contient `token`. Les routes suivantes envoient `Authorization: Bearer <token>`.

```bash
token="<colle le token ici>"
```

### 2. Créer un endpoint

Le secret HMAC n'est renvoyé qu'à la création. `GET /endpoints` ne le contient pas. `DELETE` désactive l'endpoint : les nouveaux événements ne le ciblent plus, les tentatives déjà en file continuent.

```bash
curl -s -X POST http://127.0.0.1:8080/endpoints \
  -H "authorization: Bearer $token" \
  -H 'content-type: application/json' \
  -d '{"url":"http://127.0.0.1:9000/webhook","event_types":["invoice.paid"]}'
```

Conserve `id` et `secret`. Un endpoint peut s'abonner à plusieurs types, ou à `"*"` pour tout recevoir.

```bash
curl -s http://127.0.0.1:8080/endpoints \
  -H "authorization: Bearer $token"
```

### 3. Recevoir le webhook

Dans un autre terminal, un petit serveur qui renvoie 204 et affiche la signature :

```bash
python - <<'PY'
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = self.rfile.read(length)
        print("event", self.headers.get("X-Webhook-Event"))
        print("signature", self.headers.get("X-Webhook-Signature"))
        print("body", body.decode())
        self.send_response(204)
        self.end_headers()

ThreadingHTTPServer(("127.0.0.1", 9000), Handler).serve_forever()
PY
```

Sous PowerShell, enregistre le même script dans `receiver.py` puis lance `python receiver.py`.

### 4. Envoyer un événement

```bash
curl -s -X POST http://127.0.0.1:8080/events \
  -H "authorization: Bearer $token" \
  -H 'content-type: application/json' \
  -d '{"type":"invoice.paid","payload":{"id":"inv_1","amount":1200}}'
```

La réponse `202` confirme la mise en file. Seuls les endpoints actifs abonnés à `invoice.paid` (ou à `*`) reçoivent une tentative.

### 5. Lire l'historique

```bash
curl -s http://127.0.0.1:8080/events/<event_id> \
  -H "authorization: Bearer $token"
```

Chaque élément de `attempts` porte le statut (`pending`, `failed`, `success`, `exhausted`), le code HTTP, le nombre de tentatives et la date de la prochaine échéance.

### 6. Forcer un replay

```bash
curl -s -X POST http://127.0.0.1:8080/events/<event_id>/replay \
  -H "authorization: Bearer $token" \
  -H 'content-type: application/json' \
  -d '{}'
```

Les tentatives des endpoints encore actifs repassent en `pending`, avec un budget de retries remis à zéro, et sont dues immédiatement. Un endpoint ajouté après l'événement est inclus.

### Simuler un endpoint qui échoue

Remplace le serveur Python par une réponse `500` :

```python
self.send_response(500)
self.end_headers()
```

Puis renvoie l'événement. `GET /events/<id>` montre `status: "failed"`, `http_status: 500` et `attempt_count: 1`. La prochaine tentative est dans 30 secondes, puis 2 minutes, 10 minutes, 1 heure et 6 heures. Après 6 échecs le statut devient `exhausted`.

Pour ne pas attendre, `POST /events/<id>/replay` relance tout de suite. Un timeout ou une erreur réseau compte aussi comme un échec (`http_status` reste `null`).

## Vérifier la signature côté client

Chaque `POST` contient :

| En-tête | Rôle |
| --- | --- |
| `X-Webhook-Id` | Identifiant de l'événement |
| `X-Webhook-Event` | Type, par exemple `invoice.paid` |
| `X-Webhook-Timestamp` | Secondes Unix de la signature |
| `X-Webhook-Signature` | `t=<timestamp>,v1=<hex>` |
| `Content-Type` | `application/json` |

Le corps est le payload JSON, compact, tel qu'il est envoyé. Le MAC est HMAC-SHA256 de la chaîne UTF-8 `"{timestamp}.{corps brut}"`, clé = le secret `whsec_...` reçu à la création. Compare le hex en temps constant, et refuse un timestamp de plus de 5 minutes pour limiter le rejeu.

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn verify(secret: &str, body: &[u8], header: &str, now_unix: i64) -> bool {
    let mut timestamp = None;
    let mut presented = None;
    for part in header.split(',') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("t=") {
            timestamp = value.parse::<i64>().ok();
        } else if let Some(value) = part.strip_prefix("v1=") {
            presented = Some(value);
        }
    }
    let (Some(timestamp), Some(presented)) = (timestamp, presented) else {
        return false;
    };
    if (now_unix - timestamp).abs() > 300 {
        return false;
    }
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("hmac");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());
    if expected.len() != presented.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(presented.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}
```

Vecteur de test : secret `whsec_test`, timestamp `1700000000`, corps `{"ok":true}` →  
`t=1700000000,v1=85876387ad9d6be57a04653bc0729da757049f58afb10ba6cac3bedaecf4fda3`.

Les redirections HTTP ne sont pas suivies : un `3xx` est un échec, pour ne pas renvoyer le corps signé vers un autre hôte.

## Configuration

| Variable | Défaut | Rôle |
| --- | --- | --- |
| `DATABASE_URL` | — | URL MySQL `mysql://...` |
| `JWT_SECRET` | — | Secret HS256, 32 caractères minimum |
| `JWT_TTL_SECS` | `86400` | Durée de vie du jeton |
| `BIND_ADDR` | `0.0.0.0:8080` | Adresse d'écoute |
| `WORKER_POLL_INTERVAL_MS` | `1000` | Période du worker |
| `WORKER_BATCH_SIZE` | `50` | Livraisons lues par passage |
| `WORKER_CONCURRENCY` | `20` | Livraisons HTTP simultanées |
| `HTTP_TIMEOUT_SECS` | `10` | Timeout d'un POST sortant |
| `MAX_DELIVERY_ATTEMPTS` | `6` | Budget avant `exhausted` |
| `CLAIM_LEASE_SECS` | `60` | Bail anti double-envoi, supérieur au timeout |
| `RUST_LOG` | `webhook_relay=info,tower_http=info` | Filtre de logs |

## Déploiement

L'image est multi-étapes. Elle compile en mode hors-ligne SQLx, donc sans MySQL pendant le build.

```bash
docker build -t webhook-relay .
docker run --rm -p 8080:8080 \
  -e DATABASE_URL=mysql://user:password@host:3306/webhook_relay \
  -e JWT_SECRET=change-me-to-a-long-random-string-at-least-32-bytes \
  webhook-relay
```

Le processus applique les migrations au démarrage et s'arrête proprement sur `SIGTERM`.

## Modèle

- `users` : compte client de l'API (email, hash Argon2id).
- `endpoints` : URL, types souscrits (JSON), secret HMAC, actif ou non. Le secret reste en base pour signer ; l'API ne le réaffiche pas.
- `events` : type, payload JSON, date de réception.
- `delivery_attempts` : une ligne par couple événement / endpoint. Statut, code HTTP, nombre de tentatives, dernière tentative, prochaine échéance.

Statuts :

- `pending` : jamais tenté, ou relancé par un replay.
- `failed` : dernier essai en échec, une nouvelle date est planifiée.
- `success` : réponse `2xx`.
- `exhausted` : le budget de tentatives est consommé.

Le worker ne prend que `pending` et `failed` dont `next_attempt_at` est passée. Il pose d'abord un bail (`next_attempt_at` repoussé de `CLAIM_LEASE_SECS`) pour qu'un second passage ne renvoie pas la même livraison pendant l'appel HTTP.

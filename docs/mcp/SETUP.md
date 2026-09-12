# bsl-analyzer: установка с нуля (LSP + MCP)

Пошаговый **сценарий** «от пустого репозитория 1С до работающих LSP и MCP». Рассчитан
на то, что AI-ассистент (Claude Code и т.п.) или человек проходит его один раз для
конкретного проекта.

`bsl-analyzer` — это LSP + MCP сервер для 1C:Enterprise:

- **LSP** (Claude Code, Linux/macOS): навигация, диагностики, автодополнение в редакторе;
- **MCP**: два профиля — `reference` (справка платформы/ИТС) и `workspace` (анализ
  конкретной конфигурации: код, метаданные, граф вызовов, диагностики, SDBL, live-база).

Глубокие справочники, на которые ссылается этот сценарий:

- установка MCP-профилей и `mcp install` — [`README.md`](README.md);
- список инструментов и расширение 1С — [`TOOLS_AND_EXTENSION.md`](TOOLS_AND_EXTENSION.md).

Минимальная версия: **0.2.x** (команды `mcp` и `extension`, инструменты `graph` и
`diagnostics`).

> Этот сценарий самодостаточен: всё, что нужно, скачивается из публичных GitHub Releases.

---

## ⚠️ Агенту: это сценарий с тремя обязательными развилками

Не проходи happy path молча. В установке есть **три решения, которые должен принять
пользователь** — на каждом ОСТАНОВИСЬ и спроси, не выбирай за него:

1. **Развилка A — семантический поиск (embeddings).** Шаг 4.
2. **Развилка B — живая база 1С (live-инструменты).** Шаг 6.
3. **Развилка C — MCP стандартов v8std.** Шаг 7.

Setup НЕ считается завершённым, пока пользователь не ответил на все три и пока не выдан
**финальный чеклист** (шаг 9). Если на развилку ответили «нет» — это валидный ответ, но
он должен быть явно зафиксирован в чеклисте, а не пропущен.

---

## Шаг 1. Лаунчер в PATH (НЕ app, НЕ зеркало)

Ставим **лаунчер** `bsl-analyzer` — тонкую обёртку, которая сама скачивает и
авто-обновляет рабочий бинарник (`bsl-analyzer-app`) в `~/.bsl-analyzer/bin/`. Лаунчер —
это ваша единственная точка входа в PATH.

> **Типичная ошибка установки — не делайте так:**
> - ❌ не качайте `bsl-analyzer-app-*` напрямую — это рабочий бинарник без авто-обновления;
> - ✅ качайте ассет **лаунчера** `bsl-analyzer-<platform>` из **GitHub Releases**.

### Linux / macOS

```bash
# Linux x86_64 -> bsl-analyzer-linux-amd64
# macOS Apple Silicon -> bsl-analyzer-darwin-arm64
ASSET="bsl-analyzer-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m | sed 's/x86_64/amd64/;s/aarch64/arm64/;s/arm64/arm64/')"
mkdir -p ~/.local/bin
curl -fsSL -o ~/.local/bin/bsl-analyzer \
  "https://github.com/itrous/bsl-analyzer/releases/latest/download/${ASSET}"
chmod +x ~/.local/bin/bsl-analyzer
```

Убедитесь, что `~/.local/bin` есть в `PATH`:

```bash
echo $PATH | tr ':' '\n' | grep -q "$HOME/.local/bin" || echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
```

### Windows (PowerShell)

```powershell
irm https://raw.githubusercontent.com/itrous/bsl-analyzer/develop/scripts/install.ps1 | iex
```

Инсталлятор ставит именно лаунчер, сверяет SHA-256 с `checksums.txt` релиза и сам
добавляет каталог установки в пользовательский `PATH`.

### Проверка (обязательная) — лаунчер и app по отдельности

```bash
bsl-analyzer --launcher-version   # версия лаунчера
bsl-analyzer --launcher-update    # скачать/обновить рабочий app-бинарник
bsl-analyzer --version            # версия app (лаунчер пере-исполняет app); ждём 0.2.x+
```

Если `--version` отдаёт версию `0.2.x` или новее — лаунчер скачал app и связка работает.

---

## Шаг 2. LSP-плагин для Claude Code (Linux/macOS)

Пропустите для Cursor и Windows — там используется только MCP.

> IDE запускает серверы в урезанном окружении, где `~/.local/bin` может не быть в PATH.
> Узнайте полный путь к **лаунчеру** и используйте его в `command`: `which bsl-analyzer`.

```bash
mkdir -p ~/.claude/plugins/bsl-lsp/.claude-plugin
```

`~/.claude/plugins/bsl-lsp/.claude-plugin/plugin.json`:

```json
{
  "name": "bsl-lsp",
  "description": "BSL (1C:Enterprise) language support via bsl-analyzer",
  "version": "0.1.0"
}
```

`~/.claude/plugins/bsl-lsp/.lsp.json` (подставьте путь из `which bsl-analyzer` — это путь
**лаунчера** `~/.local/bin/bsl-analyzer`, не app):

```json
{
  "bsl": {
    "command": "/home/<user>/.local/bin/bsl-analyzer",
    "args": ["lsp"],
    "extensionToLanguage": { ".bsl": "bsl", ".os": "bsl" }
  }
}
```

Включите плагин в `~/.claude/settings.json` (поле `enabledPlugins`):

```json
"bsl-lsp@local": true
```

---

## Шаг 3. Установка MCP + проверка пути `command`

Нативная команда сама пишет конфиг для выбранного клиента (Codex / Gemini / Claude /
Cursor) в нужный scope:

```bash
bsl-analyzer mcp install --target all --preset recommended --source-dir .
```

`recommended` ставит `reference` глобально (user) и `workspace` для текущего проекта
(project). Клиенты, scope'ы, флаги `--dry-run`/`--force` — в [`README.md`](README.md).

Для профиля `workspace` оптимизация-брокер включена по умолчанию на всех платформах,
включая Windows (транспорт — named pipe с current-user security descriptor и проверкой
личности backend'а). Принудительный прямой `stdio` — переменной `BSL_MCP_BROKER=0`.

После обновления с более старого бинарника один раз завершите уже запущенные MCP-процессы
и переподключите клиент: новый код не может исправить состояние в памяти старого процесса.
Дальше новое подключение штатно принимает совместимый кэш. Не запускайте одновременно
broker-backed и прямой `stdio`/fallback workspace-профили над одним проектом: даже краткий
прямой процесс захватывает общий `writer.lease`, и замеченный прежним backend захват
необратимо переводит его в `superseded`; восстановление — переподключение к актуальному
backend, а не ожидание возврата аренды.

### Проверка: в `command` должен стоять путь лаунчера

Запущенный через лаунчер, `mcp install` сам записывает в `command` путь **лаунчера**
(а не кэшированного app-бинарника) — так MCP-клиент продолжает ходить через
авто-обновляемый лаунчер. Убедитесь в этом:

```bash
codex mcp list
```

Ожидаемый `command`:

```
/home/<user>/.local/bin/bsl-analyzer
```

Если там `~/.bsl-analyzer/bin/bsl-analyzer-app` — значит `mcp install` был запущен не
через лаунчер (например, прямым вызовом app-бинарника). Перезапустите установку именно
через лаунчер (`bsl-analyzer mcp install …` из PATH).

> **PATH-независимость.** Путь в `command` — абсолютный, поэтому сервер стартует даже
> когда AI-клиент запускает его без `~/.local/bin` в `PATH`.

---

## Шаг 4. 🔀 Развилка A — семантический поиск (embeddings)

**Спросите пользователя: включаем семантическую составляющую поиска (`search_code`/`search_docs`)?**

- **Нет** → `search_code` работает в лексическом режиме (с пометкой
  `-- semantic skipped: … --`), плюс `find_docs`, `metadata`, `graph`,
  `diagnostics`, SDBL — **всё работает без эмбеддингов**. Переходите к шагу 5.
- **Да** → нужен доступ к OpenAI-совместимому embedder'у и переменные:

  | Переменная | Назначение | Дефолт |
  |-----------|-----------|--------|
  | `EMBEDDING_URL` | базовый URL embedder'а | — (обязательно) |
  | `EMBEDDING_MODEL` | имя модели | `Qwen/Qwen3-Embedding-0.6B` |
  | `EMBEDDING_DIM` | размерность вектора (должна совпасть с моделью) | `1024` |
  | `EMBEDDING_API_KEY` | ключ (для сервисов с Bearer-авторизацией) | — (опц.) |
  | `EMBEDDING_MAX_REQUEST_BYTES` | положительный предел одного сериализованного HTTP JSON body, байты | `1048576` (1 MiB) |
  | `EMBEDDING_PUBLISH_RETRY_BUDGET_SECS` | общий лимит повторных попыток публикации, секунды | `600` |

  Установка с прописыванием env прямо в MCP-конфиг:

  ```bash
  bsl-analyzer mcp install --target all --preset recommended --source-dir . \
    --env EMBEDDING_URL=https://your-embedder/v1 \
    --env EMBEDDING_MODEL=Qwen/Qwen3-Embedding-0.6B \
    --env EMBEDDING_DIM=1024 \
    --env EMBEDDING_PUBLISH_RETRY_BUDGET_SECS=600 \
    --env EMBEDDING_API_KEY=sk-...
  ```

  **Проверка, что семантика реально поднялась** — вызовите MCP-инструмент `search` с
  `action=status` и найдите строку `Semantic:`:

  - `Semantic: available...` — семантика работает;
  - `Semantic: not configured (set EMBEDDING_URL)` — эмбеддинги не подхватились;
  - `Semantic: failed...` / `syncing...` — не готова / строится.

  Настройка размера, ошибки значения, таймауты и откат —
  [«Размер embedding-запросов и повторные попытки»](README.md#размер-embedding-запросов-и-повторные-попытки).
  Подробности про `EMBEDDING_PROVIDER`, batch/concurrency и выбор модели —
  [`TOOLS_AND_EXTENSION.md`](TOOLS_AND_EXTENSION.md#переменные-окружения-и-prerequisites).

---

## Шаг 5. Проектный конфиг `bsl-analyzer.toml` (единый источник настроек)

Один файл в корне проекта управляет диагностиками **во всех режимах** — LSP, CLI
(`analyze`) и MCP (`diagnostics`):

```toml
[source]
root = "src/cf"

[diagnostics.parameters]
Typo = false                       # выключить правило

[diagnostics.parameters.LineLength]
maxLineLength = 150                # задать порог

[diagnostics.parameters.CyclomaticComplexity]
complexityThreshold = 30
```

После правки `bsl-analyzer.toml` MCP-сервер сам перестроит резидентную базу под новые
настройки при следующем обращении к `diagnostics`.

> Если конфиг содержит локальные пути/секреты — добавьте его в `.gitignore` (фиксируется
> в финальном чеклисте, шаг 9).

---

## Шаг 6. 🔀 Развилка B — живая база 1С (live-инструменты)

`query(action=execute)`, `execute(action=run|eval)` и `debug` работают **только** с
опубликованным HTTP-сервисом из встроенного расширения `BSL_Analyzer`.

**Спросите пользователя: нужна живая база (live-инструменты)?**

- **Нет** → пропустите; в финальном чеклисте зафиксируйте
  **«live-инструменты не настроены»**. Статический анализ (graph/diagnostics/metadata/
  search/SDBL-validate) при этом полностью работает.
- **Да** → выгрузите расширение и загрузите его **вручную** через конфигуратор:

  ```bash
  bsl-analyzer extension export -o ./bsl-extension
  ```

  Затем в конфигураторе:

  1. `Конфигурация -> Расширения конфигурации` — добавить каталог `./bsl-extension`;
  2. `Администрирование -> Публикация на веб-сервере` — включить публикацию HTTP-сервисов;
  3. назначить роль `BSL_ОсновнаяРоль` пользователю, под которым подключается MCP.

  Проверка:

  ```bash
  curl http://<хост>/<база>/hs/bsl-analyzer/version
  ```

  Полные детали (права, VRD, шаги публикации) — в
  [`TOOLS_AND_EXTENSION.md`](TOOLS_AND_EXTENSION.md#подключение-live-инструментов-к-базе-1с).

---

## Шаг 7. 🔀 Развилка C — MCP стандартов v8std

**STOP. Спросите пользователя: ставим ли публичный MCP-сервер стандартов v8std?** Не
завершайте setup, пока пользователь не ответил. Это отдельный шаг, а не «по желанию».

`v8std` (`https://ai.v8std.ru/mcp`, HTTP, без токена) дополняет наш инструментарий:

- `v8std_search` — поиск стандарта по фразе, номеру или коду диагностики;
- `v8std_get_page` / `v8std_get_related` — полный текст и связанные страницы;
- `v8std_explain_snippet` — какие стандарты применимы к фрагменту кода;
- `v8std_explain_diagnostics` — расшифровка предупреждений ACC, BSL Language Server, EDT.

Почему вместе: наш `diagnostics` находит проблему, а `v8std_explain_diagnostics`
объясняет, *почему* это нарушение и какой стандарт его требует.

- **Нет** → зафиксируйте в чеклисте «v8std: declined».
- **Да** → установите:

  **Claude Code:**

  ```bash
  claude mcp add --transport http v8std https://ai.v8std.ru/mcp
  ```

  **Codex** — `codex mcp add v8std --url https://ai.v8std.ru/mcp`, либо вручную в
  `.codex/config.toml`:

  ```toml
  [mcp_servers.v8std]
  url = "https://ai.v8std.ru/mcp"
  ```

  **Cursor** — в `.cursor/mcp.json` (поле `url`; для Antigravity — `serverUrl`):

  ```json
  { "mcpServers": { "v8std": { "url": "https://ai.v8std.ru/mcp" } } }
  ```

> **Приватность:** это публичный сервис, ему уходит текст запроса. Не отправляйте через
> него проприетарный код; для чувствительных проектов используйте локальное развёртывание.

---

## Шаг 8. Верификация инструментов

Перезапустите IDE и проверьте по слоям.

**LSP** (Claude Code, Linux/macOS): откройте `.bsl` — навигация и диагностики в редакторе.

**MCP** — инструменты сгруппированы, действие задаётся параметром `action`:

| Проверка | Вызов | Профиль |
|----------|-------|---------|
| метаданные | `metadata` (action=`info`) | workspace |
| поиск по коду | `search` (action=`search_code`) | workspace |
| граф вызовов | `graph` (action=`overview`) | workspace |
| каталог диагностик | `diagnostics` (action=`catalog`) | workspace |
| диагностики файла | `diagnostics` (action=`file`, path=…) | workspace |
| справка платформы | `syntax_help` | reference |
| справка ИТС | `its_help` | reference (нужен `NAPARNIK_TOKEN`) |
| live-запрос | `query` (action=`execute`) | workspace + расширение 1С |

Пока строятся резидентная база и граф, любой из `metadata`/`symbol_info`/`diagnostics`/`graph`
может вернуть `{"status":"loading"}` в `structuredContent` — повторите через пару секунд (см.
[`TOOLS_AND_EXTENSION.md`](TOOLS_AND_EXTENSION.md#диагностики-и-граф-проектный-конфиг-и-резидентная-база)).

---

## Шаг 9. Финальный чеклист (обязательный отчёт)

Setup завершён только когда выдан этот отчёт со всеми заполненными полями:

```
bsl-analyzer version:      <вывод `bsl-analyzer --version`>
launcher version:          <вывод `bsl-analyzer --launcher-version`>
Codex MCP servers:         <вывод `codex mcp list`; command = путь ЛАУНЧЕРА>
workspace source-dir:      <путь, переданный в `mcp install --source-dir`>
semantic search:           enabled | disabled        (развилка A)
v8std:                     installed | declined       (развилка C)
live 1C extension:         installed | skipped        (развилка B)
gitignored local config:   yes | no
```

---

## Обновление

Лаунчер сам авто-обновляет app. Принудительно:

```bash
bsl-analyzer --launcher-update        # обновить app-бинарник
bsl-analyzer --launcher-self-update   # обновить сам лаунчер
```

Сам лаунчер (точку входа в PATH) при необходимости можно перекачать из GitHub Releases —
см. шаг 1. После обновления, если используется расширение 1С, выгрузите и загрузите его
заново (шаг 6).

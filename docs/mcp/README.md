# MCP-сервер: установка и профили

Этот документ отвечает на два вопроса:

- какой MCP-профиль нужен в вашем сценарии;
- как установить его в AI-клиент без ручного редактирования конфигов.

Описание самих инструментов и подключение к базе 1С вынесены в
`docs/mcp/TOOLS_AND_EXTENSION.md`.

## Как устроены профили

`bsl-analyzer` публикует два отдельных MCP-профиля.

| Профиль | Для чего нужен | Что обычно требуется |
|--------|----------------|----------------------|
| `reference` | справка платформы, поиск по документации, `syntax_help`, `its_help` | при необходимости `EMBEDDING_URL` и `NAPARNIK_TOKEN` |
| `workspace` | поиск по коду проекта, metadata, SDBL, выполнение BSL-кода, debug | `--source-dir`; для live-инструментов ещё `--onec-url` и учётные данные |

Для нескольких ИБ используйте один `workspace`: скопируйте
`docs/mcp/onec-connections.example.json` за пределы репозитория, заполните и
передайте путь через `BSL_ONEC_CONNECTIONS_FILE`; живые инструменты выбирают
профиль параметром `connection`. Пароли задаются только переменными окружения,
указанными в реестре.

Расширение VS Code запускает языковой сервер отдельным процессом
`bsl-analyzer-app --stdio`. MCP-профиль `workspace` использует локальный
посредник и Unix-сокет, поэтому порты и сеансы с языковым сервером не делит.
Несколько MCP-клиентов одного проекта, напротив, переиспользуют один тяжёлый
сервер посредника.

Практическое правило:

- ставьте `reference`, если AI должен знать платформу и ИТС;
- ставьте `workspace`, если AI должен работать с конкретным репозиторием;
- чаще всего нужен комплект из обоих профилей.

> `reference` не использует `--source-dir` и не принимает параметры подключения
> к 1С. Live-доступ к базе относится только к `workspace`.

## Рекомендуемая установка

Самый удобный сценарий — установить оба профиля сразу:

```bash
bsl-analyzer mcp install \
  --target all \
  --preset recommended \
  --source-dir ./my-project
```

Этот preset делает следующее:

- `reference` ставится как user-scoped конфигурация;
- `workspace` ставится как project-scoped конфигурация для текущего репозитория.

Если нужны дополнительные возможности, добавьте переменные окружения и
параметры подключения:

```bash
bsl-analyzer mcp install \
  --target all \
  --preset recommended \
  --source-dir ./my-project \
  --env NAPARNIK_TOKEN=your_token \
  --env EMBEDDING_URL=http://localhost:8000/v1/embeddings \
  --env EMBEDDING_API_KEY=your_api_key \
  --onec-url http://localhost/base/hs/bsl-analyzer \
  --onec-user admin
```

## Установка по отдельности

Только глобальный профиль справки:

```bash
bsl-analyzer mcp install \
  --target all \
  --preset reference \
  --scope user \
  --env NAPARNIK_TOKEN=your_token \
  --env EMBEDDING_URL=http://localhost:8000/v1/embeddings \
  --env EMBEDDING_API_KEY=your_api_key
```

Только проектный профиль рабочего каталога:

```bash
bsl-analyzer mcp install \
  --target all \
  --preset workspace \
  --scope project \
  --source-dir ./my-project
```

Поддерживаемые scope'ы по целевым клиентам:

| Target | Поддерживаемые scope'ы | Как применяется |
|--------|-------------------------|-----------------|
| `codex` | `user`, `project` | user через CLI `codex mcp add`, project через `.codex/config.toml` |
| `gemini` | `user`, `project` | через CLI `gemini mcp add` |
| `claude` | `user`, `project`, `local` | через CLI `claude mcp add` |
| `cursor` | `user`, `project` | через merge в `mcp.json` |

## Полезные флаги `mcp install`

- `--dry-run` — показать итоговую команду или конфиг без записи на диск;
- `--force` — обновить существующую MCP-запись с тем же именем;
- `--name custom-bsl` — изменить базовое имя сервера;
- `--env KEY=value` — передать переменные окружения для MCP-процесса;
- `--onec-password` — сохранить пароль в конфиге целевого клиента.

Если пароль передаётся через `--onec-password`, он попадает в аргументы
запуска MCP-сервера. Для небезопасных контуров лучше использовать отдельные
тестовые учётные данные.

## Ручной запуск серверов

Если нужно сначала проверить профиль локально, можно запустить его вручную.
Для профиля `workspace` брокер включён по умолчанию на всех платформах, включая
Windows: транспорт — named pipe с security descriptor, ограниченным текущим
пользователем, с проверкой личности backend'а (защита от подмены имени pipe).
Принудительно вернуться на прямой `stdio` можно переменной `BSL_MCP_BROKER=0`.

Глобальный профиль справки:

```bash
bsl-analyzer mcp serve --profile reference
```

Профиль проекта:

```bash
bsl-analyzer mcp serve --profile workspace --source-dir ./my-project
```

Профиль проекта с live-доступом к базе 1С:

```bash
bsl-analyzer mcp serve \
  --profile workspace \
  --source-dir ./my-project \
  --onec-url http://localhost/base/hs/bsl-analyzer \
  --onec-user admin \
  --onec-password secret
```

### Запуск по HTTP

Локальный сервер проекта:

```bash
bsl-analyzer mcp serve \
  --profile workspace \
  --source-dir ./my-project \
  --mode http \
  --port 8021
```

Общий справочный сервер запускается без каталога проекта:

```bash
bsl-analyzer mcp serve \
  --profile reference \
  --mode http \
  --port 8020
```

Оба сервера публикуют MCP по адресу `http://127.0.0.1:<порт>/mcp`.
Состояние процесса доступно по `http://127.0.0.1:<порт>/health`. Ответ содержит
`status`, `version`, `profile`, `mode`, `host`, `port`, `pid` и
`uptime_seconds`, но не раскрывает путь проекта, параметры подключения к 1С
или секреты. Пути `/mcp` и `/health` не настраиваются.

Для доступа из локальной сети адрес и разрешённое значение `Host` задаются
явно:

```bash
bsl-analyzer mcp serve \
  --profile workspace \
  --source-dir ./my-project \
  --mode http \
  --host 0.0.0.0 \
  --port 8021 \
  --allowed-host mcp-server.local
```

Правила интерфейса:

- `--host`, `--port` и `--allowed-host` допустимы только с `--mode http`;
- для HTTP порт обязателен и должен входить в диапазон `1..65535`;
- `--host` по умолчанию равен `127.0.0.1`;
- нелокальный адрес требует хотя бы одного `--allowed-host`;
- HTTP-сервер ограничивает размер тела запроса одним мебибайтом;
- `workspace` по-прежнему требует `--source-dir`;
- `reference` не требует `--source-dir` и не принимает параметры подключения к
  1С;
- явный `--mode http` не подменяется режимом `broker`.

Запуск без `--mode http` не меняется: команды и поведение режимов `stdio`,
`broker` и `daemon` остаются прежними.

Процесс удерживает исключительную системную блокировку записи до завершения.
Поэтому второй HTTP-сервер того же проекта или профиля не запускается даже на
другом порту. Для `workspace` запись находится в
`<source-dir>/.build/bsl-analyzer-mcp-http.pid.json`, для `reference` — в
каталоге состояния приложения под именем
`bsl-analyzer/bsl-analyzer-mcp-http-reference.pid.json`. После штатной
остановки файл остаётся со `state: "stopped"`; после аварии устаревший файл
тоже может остаться, но системная блокировка освобождается автоматически.

Проверка `Host` не заменяет проверку подлинности. Не публикуйте сервер
напрямую в интернет: ограничьте доступ межсетевым экраном или частной сетью,
а TLS и проверку подлинности настройте на обратном посреднике.

Если нужна ручная интеграция в конкретный AI-клиент, обычно проще не писать
конфиг с нуля, а сначала выполнить `mcp install --dry-run` и использовать
показанную команду или сгенерированный фрагмент как образец.

## Эмбеддинги и семантический поиск

Семантическая составляющая поиска требует `EMBEDDING_URL`:

- `search(action=search_docs)` в профиле `reference`;
- семантическая ветвь `search(action=search_code)` в профиле `workspace`
  (лексическая ветвь `search_code` работает и без неё).

Если embedding-провайдер требует Bearer-авторизацию
(например, OpenRouter, OpenAI или совместимый сервис), дополнительно задайте
`EMBEDDING_API_KEY`.

Для задач семантического поиска по коду и документации разумно явно задавать
`EMBEDDING_MODEL`. Практический ориентир:

- `Qwen/Qwen3-Embedding-0.6B` — минимальная рекомендуемая модель и текущий
  дефолт в `bsl-analyzer`;
- `Qwen/Qwen3-Embedding-4B` — усиленный компромиссный вариант;
- `Qwen/Qwen3-Embedding-8B` — предпочтительный вариант, если позволяет ресурс.

Пример:

```bash
EMBEDDING_URL=https://openrouter.ai/api \
EMBEDDING_API_KEY=your_api_key \
EMBEDDING_MODEL=Qwen/Qwen3-Embedding-0.6B \
  bsl-analyzer mcp serve --profile reference
```

Если `EMBEDDING_URL` не задан, остаётся полнотекстовый `find_docs` в `reference`, а
`search_code` в `workspace` возвращает лексические результаты с пометкой
`-- semantic skipped: … --`.

Сейчас `bsl-analyzer` для embedding API поддерживает стандартный заголовок
`Authorization: Bearer ...` через `EMBEDDING_API_KEY`. Если конкретный
провайдер требует дополнительные нестандартные заголовки, это нужно учитывать
отдельно.

Если используется централизованный baseline в PostgreSQL, семантический поиск
комбинирует локальный runtime и shared baseline. Подробности — в
`docs/central-postgres-search/README.md`.

## Что читать дальше

- `docs/mcp/SETUP.md` — установка с нуля (бинарник, PATH, LSP-плагин, верификация)
- `docs/mcp/TOOLS_AND_EXTENSION.md` — доступные инструменты, prerequisites и расширение 1С
- `docs/central-postgres-search/README.md` — shared baseline и overlay для поиска

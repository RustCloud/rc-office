# rc-office — план реализации

Плагин RustCloud «Офис»: открывает и редактирует документы (`.docx`, `.xlsx`, `.pptx`, `.odt`, `.ods`,
`.odp` и др.) в Collabora Online. Collabora — **WOPI-клиент** (отдельный контейнер), плагин —
**WOPI-сервер** поверх файлового API ядра. Документ остаётся обычным файлом пользователя: версии,
квота, журнал, корзина — от ядра.

Основа в ядре уже есть (ROADMAP ядра, «Порядок работ», п. 7, API **0.2.14**):

| Что нужно офису                  | Чем даёт ядро                                                       |
|----------------------------------|---------------------------------------------------------------------|
| «Открыть в Офисе», полный экран  | `[[open_with]]` в манифесте, `setLayout`, `close`, `context.file`   |
| Доступ к одному файлу            | доступ `session`, пока открыта страница (+1 мин на сохранение)      |
| Collabora ходит к плагину сама   | `tokens.issue(file, write, seconds)`, `?access_token=` на маршрутах  |
| GetFile любого размера           | ответ `x-rustcloud-send: file:<id>` — сервер отдаёт файл потоком     |
| PutFile любого размера           | `files.write-request-body(id, if-match)` + `large_bodies = true`     |
| Не затереть чужие правки         | `If-Match` атомарно в `write-request-body` / `write-if-match`        |
| WOPI-блокировки                  | `files.lock` / `unlock` / `get-lock`, ошибка `locked(file-lock)`      |
| Адрес Collabora у каждого свой   | `"{office_url}"` в `permissions.network` и `permissions.frame`       |
| Редактор вне песочницы страницы  | `rustcloud.openFrame` (форма `POST` с токеном), `onFrameMessage`, `postToFrame` |
| `PostMessageOrigin`              | `context.origin` — адрес панели                                       |
| История                          | версии файлов (`versions`, `read-version`) — сами при каждой записи   |

Collabora CODE 26.04.5.2 уже проверена на buildserver (контейнер `collabora-test`, порт 9980, без TLS,
`aliasgroup1=http://192.168.4.131:8080`): discovery, `capabilities`, `convert-to` и сеть в обе стороны
работают. Полный цикл WOPI — задача этого плагина.

## Как это работает

```text
Браузер                       RustCloud (ядро + плагин rc-office)          Collabora
───────                       ───────────────────────────────────          ─────────
«Открыть в Офисе»  ──────────▶ панель: доступ session к файлу,
                               #/app/rc-office/edit?file=<id>
страница плагина ── fetch /session?file=<id> (через мост) ──▶ плагин:
                               1. discovery (кэш)  ──────────────────────▶ GET /hosting/discovery
                               2. urlsrc по расширению и действию (edit/view)
                               3. tokens.issue(file, write, 10 ч)
                               4. запомнить origin панели (AppData)
                 ◀── { url, token, ttl } ──
rustcloud.openFrame({ url, fields: { access_token, access_token_ttl } })
панель: iframe поверх страницы, POST-форма ──────────────────────────────▶ cool.html
                                                       ◀── WOPI с ?access_token=…
                               GET  /wopi/files/<id>            CheckFileInfo
                               GET  /wopi/files/<id>/contents   GetFile  (x-rustcloud-send)
                               POST /wopi/files/<id>            LOCK / REFRESH_LOCK / UNLOCK / GET_LOCK …
                               POST /wopi/files/<id>/contents   PutFile  (write-request-body)
PostMessage (UI_Close, Doc_ModifiedStatus…) ◀── панель ◀── iframe Collabora
```

`WOPISrc = <адрес облака для Collabora>/app/rc-office/wopi/files/<file-id>`. Он должен совпадать с
`aliasgroup1` Collabora. По умолчанию — `context.origin`, но у Collabora в compose-сети облако часто
видно по другому адресу (`http://rustcloud:8080`) — для этого настройка `wopi_base_url` (ниже).

## Манифест (черновик)

```toml
id = "rc-office"
version = "0.1.0"
api = "^0.2.14"

[name]
en = "Office"
ru = "Офис"

[permissions]
files = "opened"          # только открытый файл; "folder" — когда появится «Создать документ»/превью
files_write = true
network = ["{office_url}"]   # discovery, capabilities, convert-to
frame = ["{office_url}"]

large_bodies = true       # PutFile больше предела данных плагина

[[server_settings]]
key = "office_url"
kind = "text"
label = { en = "Collabora Online address", ru = "Адрес Collabora Online" }

[[server_settings]]
key = "wopi_base_url"
kind = "text"
label = { en = "Cloud address for Collabora (if not the public one)",
          ru = "Адрес облака для Collabora (если не публичный)" }

[[open_with]]
mime = [
  "application/vnd.oasis.opendocument.text",
  "application/vnd.oasis.opendocument.spreadsheet",
  "application/vnd.oasis.opendocument.presentation",
  "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  "application/msword", "application/vnd.ms-excel", "application/vnd.ms-powerpoint",
  "application/rtf", "text/csv",
]
page = "/edit"
write = true
layout = "full"
label = { en = "Edit in Office", ru = "Редактировать в Офисе" }

[ui]
icon = "file-text"
entry = "/app/rc-office"   # страница: состояние связи с Collabora, позже — «Создать документ»
```

Список `mime` лучше сверять с discovery: показывать «Открыть» только для того, что Collabora умеет
(сейчас `[[open_with]]` статичен — см. вопросы).

## Устройство кода

```text
rc-office/
  manifest.toml
  Cargo.toml              # cdylib, wasm32-wasip2, wit-bindgen 0.63
  wit/                    # копия plugin-api/wit ядра (app.wit)
  src/
    lib.rs                # handle(): маршрутизация
    routes/session.rs     # /session — urlsrc + токен для страницы
    wopi/
      mod.rs              # разбор X-WOPI-Override, ответы, коды
      check_file_info.rs
      contents.rs         # GetFile / PutFile
      locks.rs            # LOCK, UNLOCK, REFRESH_LOCK, GET_LOCK, UNLOCK_AND_RELOCK
    discovery.rs          # загрузка, разбор XML, кэш в storage
    backend.rs            # трейт над files/tokens/appdata — для юнит-тестов без WASM
  ui/                     # edit.html, index.html, свой JS; всё в архиве плагина, без CDN
  templates/              # (этап 4) пустые .odt/.ods/.odp/.docx/.xlsx/.pptx
  e2e/                    # Playwright на buildserver
  .forgejo/ или .github/workflows/
```

Логику WOPI держать за трейтом `Backend` (файлы, блокировки, токены, AppData), чтобы покрыть её
обычными `cargo test` на хосте: таблицы «запрос → ответ» на каждый случай протокола.

## WOPI: что и как отвечает плагин

Каждый WOPI-запрос: `tokens::current()` должен быть, `scope.file` == `<id>` из пути (ядро и так не
даст другой файл, но ответим `401`/`404` явно). Без токена — `401`.

**CheckFileInfo** `GET /wopi/files/<id>` — JSON:

- `BaseFileName`, `Size`, `Version` (= `etag`), `LastModifiedTime` (= `updated-at`, ISO 8601);
- `OwnerId`/`UserId` = UUID пользователя, `UserFriendlyName` = `display-name`;
- `UserCanWrite` = `scope.write`; `ReadOnly` = `!scope.write`;
- `SupportsLocks: true`, `SupportsUpdate: true`, `SupportsGetLock: true`;
- `UserCanNotWriteRelative: true`, `SupportsRename: false`, `UserCanRename: false` — пока нет «Сохранить
  как» и переименования (этап 7);
- `PostMessageOrigin` = origin панели (сохранён страницей в AppData при `/session`, ключ — файл);
- `HideSaveOption`/`HidePrintOption`/`DisableExport` — по умолчанию нет; `EnableShare: false`.

**GetFile** `GET /wopi/files/<id>/contents` — пустое тело и `x-rustcloud-send: file:<id>`, плюс
`X-WOPI-ItemVersion: <etag>`. Файл не проходит через память плагина.

**PutFile** `POST /wopi/files/<id>/contents` (`X-WOPI-Override: PUT`):

1. если файл заблокирован — `X-WOPI-Lock` должен совпасть, иначе `409` + `X-WOPI-Lock: <текущий>`;
   без блокировки и непустой файл — `409` (требование WOPI);
2. `X-COOL-WOPI-Timestamp` есть и ≠ текущему `updated-at` → `409` с `{"COOLStatusCode":1010}` — Collabora
   спросит пользователя, перезаписать ли; без заголовка (выбрал «перезаписать») — пишем;
3. `files::write_request_body(id, Some(etag))` — `etag` из того же `get`, поэтому проверка атомарна: если
   файл обогнали между шагами, ядро ответит `conflict` → тоже `409/1010`;
4. ответ `200` `{"LastModifiedTime": …}` + `X-WOPI-ItemVersion`.

`X-COOL-WOPI-IsAutosave`/`IsExitSave` можно игнорировать: ядро само схлопывает сохранения одного человека
чаще раза в 5 минут в одну версию.

**Блокировки** `POST /wopi/files/<id>` с `X-WOPI-Override`:

| Override            | Действие                                    | Чужая блокировка          |
|---------------------|---------------------------------------------|---------------------------|
| `LOCK`              | `files::lock(id, X-WOPI-Lock, 1800)`        | `409` + `X-WOPI-Lock`     |
| `REFRESH_LOCK`      | тот же `lock` (продлевает)                   | `409` + `X-WOPI-Lock`     |
| `UNLOCK`            | `files::unlock`                              | `409` + `X-WOPI-Lock`     |
| `GET_LOCK`          | `files::get-lock` → `X-WOPI-Lock` (пусто — нет) | —                     |
| `UNLOCK_AND_RELOCK` | сверить `X-WOPI-OldLock`, взять новый        | `409` + `X-WOPI-Lock`     |

30 минут — срок WOPI; у ядра до часа. Блокировка другого плагина (`app-id` ≠ `rc-office`) — тоже `409`,
`X-WOPI-Lock` пустой и `X-WOPI-LockFailureReason` с понятным текстом. Collabora при совместной правке
держит одну блокировку на документ — блокировки ядра на приложение, а не на пользователя, так что это
не мешает.

Неизвестный override — `501`. `PUT_RELATIVE`, `RENAME_FILE`, `DELETE`, `PUT_USER_INFO` — `501` до этапа 7.

**Discovery.** `GET {office_url}/hosting/discovery` через `http::fetch`, кэш в хранилище плагина
(`discovery.xml` + адрес и время получения — данные общие, не пользовательские). Обновлять раз в час,
при смене `office_url` и при ошибке открытия. Разбор XML — `quick-xml` (собирается под wasm32-wasip2).
Из него: `urlsrc` для `(ext, action)` с `edit`/`view`, ключи proof (этап 6). К `urlsrc` добавить
`WOPISrc`, `lang` (`context.locale`), `ui_theme=dark` по `context.theme`.

**Страница `/edit`.** Берёт `context.file`, зовёт `/session`, открывает `openFrame`. Слушает
`onFrameMessage`: `App_LoadingStatus` (`Frame_Ready` → `postToFrame(Host_PostmessageReady)`,
`Document_Loaded` → убрать заставку), `UI_Close` → `closeFrame()` + `rustcloud.close()`,
`Doc_ModifiedStatus` — по желанию. Ошибки (`office_url` не задан, Collabora не отвечает, формат не
поддерживается) — своим экраном с понятным текстом, не пустым фреймом.

## Этапы

Каждый этап — рабочий плагин; версия плагина растёт с каждым пушем (manifest + Cargo, CI сверяет).

0. **Каркас.** `Cargo.toml`, `wit/`, манифест, `handle` с маршрутизацией, CI собирает артефакт
   «папка с `manifest.toml` + `app.wasm` + `ui/`» (ставится как есть). Страница `/app/rc-office` —
   состояние: задан ли `office_url`, отвечает ли discovery, версия Collabora (`/hosting/capabilities`).
1. **Просмотр.** `open_with` с `write = false` пока, `/session`, CheckFileInfo, GetFile, `openFrame`.
   Проверка на стенде 192.168.4.131 + `collabora-test`: `.odt` и `.docx` открываются.
2. **Правка.** Токен на запись, PutFile с timestamp/`If-Match`, блокировки. Проверить: сохранение,
   новая версия в окне «Версии», замок в списке файлов на время правки и его снятие, конфликт при
   записи файла через панель во время правки, большой файл (> 16 МБ) через `large_bodies`, повторное
   открытие после перезагрузки страницы.
3. **Интеграция с панелью.** PostMessage (закрытие, загрузка, ошибки), язык и тема, понятные сообщения
   «файл заблокирован другим приложением», «только чтение».
4. **Создать документ.** Пустые шаблоны в архиве плагина (`include_bytes!`, несколько КБ),
   страница `/app/rc-office`: «Новый текст / таблица / презентация» → `requestAccess` к папке →
   `create-file` → открыть. Требует `files = "folder"` (новое право — админ подтвердит при обновлении).
   Лучше — пункт «Создать» в меню файлов ядра (см. вопросы).
5. **Совместная правка.** Два пользователя на одном файле (общий через «Поделиться»): оба попадают в
   одну сессию Collabora, сохранения не теряются, `UserFriendlyName` видно у курсоров.
6. **Proof keys.** Проверять `X-WOPI-Proof`/`ProofOld` + `X-WOPI-TimeStamp` по ключу из discovery
   (RSA/SHA-256, крейт `rsa`) — защита, если токен утёк. Токены и так подписаны и привязаны к файлу,
   поэтому не в первых этапах.
7. **«Сохранить как», переименование** — `PUT_RELATIVE`, `RENAME_FILE`. Нужна поддержка ядра (ниже).
8. **Превью и PDF** через `/cool/convert-to/{pdf,png}` — фоновые задачи `on = ["file.created",
   "file.changed"]`; миниатюры в AppData. Нужна поддержка ядра, чтобы список файлов их показывал.
9. **Установка.** `compose.yml` с Collabora отдельным сервисом (образ ~1,8 ГБ, ~770 МБ памяти без
   документов), `aliasgroup1`, свой ЦС внутри контейнера Collabora при HTTPS, `frame_ancestors`;
   README для администратора: настройки `office_url`, `wopi_base_url`.

## Тесты

- **Юнит** (`cargo test` на хосте, без WASM): WOPI-обработчики на фейковом `Backend` — каждая строка
  таблиц выше, разбор discovery на сохранённом XML реальной Collabora, сборка `urlsrc`.
- **WOPI validator** Microsoft (`wopi-validator-core`, .NET, в Docker на buildserver) — прогон по
  протоколу против стенда, хотя бы групп `BaseWopiViewing`, `Locks`, `EditFlows`.
- **E2E** на buildserver: Playwright (как в rc-photo), свежий контейнер сервера из HEAD ядра +
  `collabora-test`; сценарии: открыть, напечатать текст, закрыть, проверить новую версию файла;
  конфликт; два пользователя.

## Что может понадобиться от ядра

1. **Создание файлов из меню «Создать»** (`[[new_file]]` в манифесте: имя, mime, шаблон) — иначе
   плагину нужен доступ к папке ради одного нового файла.
2. **«Сохранить как» / переименование** для токена: сейчас токен видит ровно один файл, а
   `PUT_RELATIVE` создаёт соседний в той же папке; в WIT нет `rename`.
3. **Превью от плагина** в списке файлов (провайдер миниатюр по mime) — для этапа 8.
4. **Динамический `open_with`** — список форматов из discovery, а не из манифеста. Не обязательно.

## Открытые вопросы

- **CI и репозиторий.** Репо на GitHub (`RustCloud/rc-office`) — CI на GitHub Actions, или зеркалить в
  Forgejo, как rc-photo? Папка пока не git-репозиторий.
- **`PostMessageOrigin` в CheckFileInfo.** Origin панели известен странице, а CheckFileInfo зовёт
  Collabora. План — страница сохраняет его в AppData при `/session`; проверить, что AppData доступна
  вызову по токену (по коду `app_routes.rs` — да, от имени пользователя токена). Альтернатива — ядро
  отдаёт `public_url` в `settings` или в запрос.
- **`x-rustcloud-send` для вызова по токену** — по коду должно работать (пользователь токена), проверить
  на стенде с файлом больше 16 МБ.
- **Общие файлы.** Работает ли «Открыть в …» на файле, которым поделились, и чей это `OwnerId` —
  для этапа 5.
- **OnlyOffice.** Тоже умеет WOPI; держим код WOPI общим, но проверяем только Collabora. Нужен ли
  OnlyOffice вообще?
- **Срок токена.** 10 ч на сессию редактора (максимум ядра 12 ч) — подходит? После истечения Collabora
  попросит перезагрузить документ.

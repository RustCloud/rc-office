// Texts of the pages, by the panel's language (context.locale).
const TEXTS = {
  en: {
    title: 'Office',
    lead: 'Documents, spreadsheets and presentations of your files open in Collabora Online.',
    how: 'In Files, open a document’s menu and choose “Edit in Office”, or double-click it.',
    status: 'Connection to Collabora Online',
    checking: 'Checking…',
    address: 'Address',
    wopiBase: 'Cloud address for Collabora',
    wopiBaseDefault: 'the address of the panel',
    state: 'State',
    reachable: 'answers',
    unreachable: 'does not answer',
    version: 'Version',
    formats: 'Edits',
    notConfigured:
      'The Office is not set up yet: the administrator enters the address of Collabora Online in Administration → Plugins → Office.',
    recheck: 'Check again',
    opening: (name) => `Opening ${name}…`,
    starting: 'Starting the editor…',
    retry: 'Try again',
    close: 'Close',
    readOnly: 'Open read-only',
    errors: {
      'no-file': 'No document to open. Open one from Files: its menu → “Edit in Office”.',
      'no-origin': 'This version of the cloud’s panel can’t show the editor. Update RustCloud.',
      'no-office-url':
        'The Office is not set up: the administrator enters the address of Collabora Online in Administration → Plugins → Office.',
      'office-unreachable': 'Collabora Online does not answer the cloud’s server.',
      'office-invalid': 'The address of Collabora Online answers, but not as Collabora.',
      unsupported: 'Collabora Online does not open files of this type.',
      'not-found': 'The document is not open for the Office any more. Open it again from Files.',
      'frame-failed': 'The panel refused to show the editor.',
      'frame-timeout':
        'The editor did not load in the browser. Collabora Online must be reachable from this browser at its address and allow being embedded in the cloud (frame_ancestors).',
      'load-failed': 'Collabora Online could not open the document.',
      failed: 'Something went wrong.',
    },
    lockedBy: (app) =>
      `The document is being edited in the app “${app}”. It can be opened read-only until that app is done with it.`,
  },
  ru: {
    title: 'Офис',
    lead: 'Документы, таблицы и презентации из ваших файлов открываются в Collabora Online.',
    how: 'В «Файлах» откройте меню документа и выберите «Редактировать в Офисе» — или дважды щёлкните по нему.',
    status: 'Связь с Collabora Online',
    checking: 'Проверяем…',
    address: 'Адрес',
    wopiBase: 'Адрес облака для Collabora',
    wopiBaseDefault: 'адрес панели',
    state: 'Состояние',
    reachable: 'отвечает',
    unreachable: 'не отвечает',
    version: 'Версия',
    formats: 'Редактирует',
    notConfigured:
      'Офис ещё не настроен: администратор указывает адрес Collabora Online в «Администрирование → Плагины → Офис».',
    recheck: 'Проверить ещё раз',
    opening: (name) => `Открываем ${name}…`,
    starting: 'Запускаем редактор…',
    retry: 'Повторить',
    close: 'Закрыть',
    readOnly: 'Открыть для чтения',
    errors: {
      'no-file': 'Нет документа. Откройте его из «Файлов»: меню файла → «Редактировать в Офисе».',
      'no-origin': 'Эта версия панели облака не умеет показывать редактор. Обновите RustCloud.',
      'no-office-url':
        'Офис не настроен: администратор указывает адрес Collabora Online в «Администрирование → Плагины → Офис».',
      'office-unreachable': 'Collabora Online не отвечает серверу облака.',
      'office-invalid': 'По адресу Collabora Online отвечает что-то другое.',
      unsupported: 'Collabora Online не открывает файлы этого типа.',
      'not-found': 'Документ больше не открыт для Офиса. Откройте его заново из «Файлов».',
      'frame-failed': 'Панель не разрешила показать редактор.',
      'frame-timeout':
        'Редактор не загрузился в браузере. Collabora Online должна быть доступна из этого браузера по своему адресу и разрешать встраивание в облако (frame_ancestors).',
      'load-failed': 'Collabora Online не смогла открыть документ.',
      failed: 'Что-то пошло не так.',
    },
    lockedBy: (app) =>
      `Документ сейчас редактируется в приложении «${app}». Пока оно не закончит, его можно открыть только для чтения.`,
  },
};

/** Texts for a locale, English if there are none. */
function texts(locale) {
  return TEXTS[locale] ?? TEXTS.en;
}

/** An element with attributes and children (text or elements). */
function h(tag, attrs = {}, ...children) {
  const element = document.createElement(tag);
  for (const [name, value] of Object.entries(attrs)) {
    if (name === 'onclick') element.addEventListener('click', value);
    else if (value !== null && value !== undefined && value !== false) element.setAttribute(name, value);
  }
  for (const child of children.flat()) {
    if (child !== null && child !== undefined && child !== false) {
      element.append(child instanceof Node ? child : String(child));
    }
  }
  return element;
}

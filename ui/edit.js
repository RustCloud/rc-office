// The editor page (#/app/rc-office/edit?file=<id>): asks the plugin for the
// editor's address and a token, and has the panel show Collabora Online over
// this page. Collabora's PostMessage API reaches it through the panel.
(async () => {
  const context = await rustcloud.context;
  const t = texts(context.locale);
  document.documentElement.dataset.theme = context.theme;
  document.documentElement.lang = context.locale;
  const app = document.getElementById('app');

  /** Collabora must say it is there by then, or the browser can't reach it. */
  const READY_MS = 45000;
  let readyTimer = null;
  let frameOpen = false;

  const file = context.file ?? (() => {
    const id = new URLSearchParams(location.search).get('file');
    return id ? { id, name: '', write: true } : null;
  })();

  /**
   * The panel's colours for Collabora (`css_variables`: `--co-name=value;…`),
   * from this page's tokens, which follow the panel's theme.
   */
  function cssVariables() {
    const style = getComputedStyle(document.documentElement);
    const token = (name) => style.getPropertyValue(name).trim();
    const variables = {
      '--co-primary-element': token('--accent'),
      '--co-text-accent': token('--accent'),
      '--co-primary-text': token('--accent-fg'),
      '--co-primary-light': token('--card'),
      '--co-primary-element-light': token('--card'),
      '--co-color-warning': token('--warn'),
      '--co-border-radius': '8px',
    };
    return Object.entries(variables)
      .filter(([, value]) => value)
      .map(([name, value]) => `${name}=${value}`)
      .join(';');
  }

  function screen(...children) {
    app.replaceChildren(h('div', { class: 'box' }, ...children));
  }

  function loading(text) {
    screen(h('div', { class: 'spinner', role: 'progressbar', 'aria-label': text }), h('p', {}, text));
  }

  function closeButton() {
    return h('button', { onclick: () => rustcloud.close() }, t.close);
  }

  function fail(code, detail, retry = true) {
    clearTimeout(readyTimer);
    if (frameOpen) {
      rustcloud.closeFrame();
      frameOpen = false;
    }
    screen(
      h('p', {}, t.errors[code] ?? t.errors.failed),
      detail ? h('p', { class: 'detail' }, detail) : null,
      h(
        'div',
        { class: 'actions' },
        retry ? h('button', { class: 'primary', onclick: () => open({ refresh: true }) }, t.retry) : null,
        closeButton(),
      ),
    );
  }

  async function open({ refresh = false, readOnly = false } = {}) {
    loading(t.opening(file.name || ''));
    const query = new URLSearchParams({ file: file.id, origin: context.origin, lang: context.locale, theme: context.theme });
    if (!file.write || readOnly) query.set('write', '0');
    if (refresh) query.set('refresh', '1');
    let answer;
    try {
      const response = await rustcloud.fetch(`/session?${query}`);
      answer = await response.json().catch(() => ({}));
      if (!response.ok) return fail(answer.error ?? 'failed', answer.message, answer.error !== 'unsupported');
    } catch (err) {
      return fail('failed', String(err.message || err));
    }
    if (answer.locked_by && !readOnly) {
      screen(
        h('p', {}, t.lockedBy(answer.locked_by)),
        h(
          'div',
          { class: 'actions' },
          h('button', { class: 'primary', onclick: () => open({ readOnly: true }) }, t.readOnly),
          closeButton(),
        ),
      );
      return;
    }
    loading(t.starting);
    try {
      await rustcloud.openFrame({
        url: answer.url,
        fields: {
          ...(answer.fields ?? {}),
          css_variables: cssVariables(),
          access_token: answer.token,
          access_token_ttl: String(answer.ttl),
        },
      });
      frameOpen = true;
    } catch (err) {
      return fail('frame-failed', String(err.message || err));
    }
    clearTimeout(readyTimer);
    readyTimer = setTimeout(() => fail('frame-timeout', answer.url.split('?')[0]), READY_MS);
  }

  /** Collabora's messages: JSON text `{ MessageId, SendTime, Values }`. */
  rustcloud.onFrameMessage((data) => {
    let message = data;
    if (typeof data === 'string') {
      try {
        message = JSON.parse(data);
      } catch {
        return;
      }
    }
    if (!message || typeof message !== 'object') return;
    const values = message.Values ?? {};
    switch (message.MessageId) {
      case 'App_LoadingStatus':
        if (values.Status === 'Frame_Ready') {
          clearTimeout(readyTimer);
          rustcloud.postToFrame(JSON.stringify({ MessageId: 'Host_PostmessageReady', SendTime: Date.now(), Values: {} }));
        } else if (values.Status === 'Document_Loaded') {
          clearTimeout(readyTimer);
        } else if (values.Status === 'Failed') {
          fail('load-failed', values.Reason ?? values.errorMsg);
        }
        break;
      case 'Action_Load_Resp':
        if (values.success === false) fail('load-failed', values.errorMsg ?? values.errorType);
        break;
      case 'UI_Close':
        clearTimeout(readyTimer);
        frameOpen = false;
        rustcloud.closeFrame();
        rustcloud.close();
        break;
      default:
        break;
    }
  });

  if (!file) return fail('no-file', null, false);
  if (!context.origin || !rustcloud.openFrame) return fail('no-origin', null, false);
  open();
})();

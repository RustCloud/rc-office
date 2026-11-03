// The Office's own page: how to open documents, and whether Collabora Online answers.
(async () => {
  const context = await rustcloud.context;
  const t = texts(context.locale);
  document.documentElement.dataset.theme = context.theme;
  document.documentElement.lang = context.locale;
  document.title = t.title;
  const app = document.getElementById('app');

  const statusCard = h('section', { class: 'card', 'aria-live': 'polite' });
  app.append(h('h1', {}, t.title), h('p', { class: 'muted' }, t.lead), h('p', {}, t.how), statusCard);

  async function check() {
    statusCard.replaceChildren(h('h2', {}, t.status), h('p', { class: 'muted' }, t.checking));
    let status;
    try {
      const response = await rustcloud.fetch('/status');
      status = await response.json();
    } catch (err) {
      status = { configured: true, reachable: false, error: String(err.message || err) };
    }
    const recheck = h('button', { onclick: check }, t.recheck);
    if (!status.configured) {
      statusCard.replaceChildren(h('h2', {}, t.status), h('p', { class: 'warn' }, t.notConfigured), recheck);
      return;
    }
    const rows = [
      [t.address, h('code', {}, status.office_url ?? '')],
      [
        t.wopiBase,
        status.wopi_base_url ? h('code', {}, status.wopi_base_url) : h('span', { class: 'muted' }, t.wopiBaseDefault),
        status.wopi_base_is_office ? h('div', { class: 'warn' }, t.wopiBaseIsOffice) : null,
      ],
      [
        t.state,
        status.reachable
          ? h('span', { class: 'ok' }, '● ', t.reachable)
          : h('span', { class: 'bad' }, '● ', t.unreachable, status.error ? h('div', { class: 'detail' }, status.error) : null),
      ],
    ];
    if (status.version) rows.push([t.version, `${status.product ?? 'Collabora Online'} ${status.version}`]);
    if (status.editable?.length) {
      rows.push([t.formats, h('div', { class: 'formats' }, status.editable.map((ext) => h('span', {}, ext)))]);
    }
    statusCard.replaceChildren(
      h('h2', {}, t.status),
      h('dl', {}, rows.map(([name, ...value]) => [h('dt', {}, name), h('dd', {}, value)])),
      h('div', { style: 'margin-top: 16px' }, recheck),
    );
  }

  check();
})();

const access = document.querySelector('#access');
const accessForm = document.querySelector('#access-form');
const tokenInput = document.querySelector('#admin-token');
const dashboard = document.querySelector('#dashboard');
const fleet = document.querySelector('#fleet');
const message = document.querySelector('#message');
let token = '';

function element(tag, className = '', value = '') {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (value) node.textContent = value;
  return node;
}

function relativeTime(seconds) {
  if (seconds < 10) return 'just now';
  if (seconds < 60) return `${seconds}s ago`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
  return `${Math.floor(seconds / 86400)}d ago`;
}

function badge(label, stale = false) {
  const state = stale ? 'stale' : label;
  return element('span', `status ${state}`, stale ? 'not seen recently' : label);
}

function renderAgent(id, record, now) {
  const heartbeat = record.heartbeat;
  const age = Math.max(0, now - record.last_seen_unix_secs);
  const stale = age > 90;
  const card = element('article', 'agent');
  const main = element('div', 'agent-main');
  const top = element('div', 'agent-top');
  const idLabel = element('span', 'agent-id', id);
  top.append(idLabel, badge(heartbeat.health, stale));
  const name = element('h3', '', `Agent ${id.slice(0, 8)}`);
  const meta = element('div', 'agent-meta', `v${heartbeat.version} · ${heartbeat.os} / ${heartbeat.arch} · ${relativeTime(age)}`);
  const history = element('div', 'history');
  history.setAttribute('aria-label', `${record.history.length} recent health samples`);
  for (const sample of record.history.slice(-40)) {
    const bar = element('span', sample.health === 'healthy' ? '' : 'degraded');
    bar.title = `${sample.health} · ${new Date(sample.at_unix_secs * 1000).toLocaleString()}`;
    history.append(bar);
  }
  main.append(top, name, meta, history);

  const repos = element('div', 'agent-repos');
  repos.append(element('div', 'repo-title', `REPOSITORIES / ${heartbeat.repositories.length}`));
  for (const repository of heartbeat.repositories.slice(0, 30)) {
    const row = element('div', 'repo');
    const repoId = element('span', 'repo-id', repository.id.slice(0, 18));
    repoId.title = repository.id;
    row.append(repoId, badge(repository.health));
    repos.append(row);
  }
  if (heartbeat.repositories.length > 30) {
    repos.append(element('p', 'repo-count', `+ ${heartbeat.repositories.length - 30} more repositories`));
  }
  if (!heartbeat.repositories.length) {
    repos.append(element('p', 'repo-count', 'No repositories registered'));
  }
  card.append(main, repos);
  return card;
}

function render(data) {
  const entries = Object.entries(data.agents || {})
    .sort((left, right) => right[1].last_seen_unix_secs - left[1].last_seen_unix_secs);
  const now = data.observed_at_unix_secs;
  let recent = 0;
  let healthy = 0;
  let attention = 0;
  for (const [, record] of entries) {
    if (now - record.last_seen_unix_secs <= 90) recent++;
    for (const repository of record.heartbeat.repositories) {
      if (repository.health === 'healthy') healthy++;
      else attention++;
    }
  }
  document.querySelector('#agent-count').textContent = entries.length;
  document.querySelector('#recent-count').textContent = recent;
  document.querySelector('#healthy-count').textContent = healthy;
  document.querySelector('#attention-count').textContent = attention;
  document.querySelector('#updated-at').textContent = `UPDATED ${new Date(now * 1000).toLocaleTimeString()}`;
  fleet.replaceChildren(...entries.map(([id, record]) => renderAgent(id, record, now)));
  document.querySelector('#empty').hidden = entries.length > 0;
}

async function load() {
  if (!token) return;
  try {
    const response = await fetch('/api/v1/agents', {
      headers: { Authorization: `Bearer ${token}` },
      cache: 'no-store',
    });
    if (response.status === 401) {
      token = '';
      access.hidden = false;
      dashboard.hidden = true;
      throw new Error('That administrator token was not accepted.');
    }
    if (!response.ok) throw new Error(`Hub request failed (${response.status}).`);
    render(await response.json());
    access.hidden = true;
    dashboard.hidden = false;
    message.textContent = '';
  } catch (error) {
    message.textContent = error.message;
  }
}

accessForm.addEventListener('submit', event => {
  event.preventDefault();
  token = tokenInput.value.trim();
  tokenInput.value = '';
  load();
});
document.querySelector('#refresh').addEventListener('click', load);
document.querySelector('#disconnect').addEventListener('click', () => {
  token = '';
  dashboard.hidden = true;
  access.hidden = false;
  message.textContent = '';
  tokenInput.focus();
});

function updateClock() {
  document.querySelector('#clock').textContent = new Date().toLocaleTimeString([], {
    hour: '2-digit', minute: '2-digit',
  });
}
updateClock();
setInterval(updateClock, 30_000);
setInterval(load, 10_000);

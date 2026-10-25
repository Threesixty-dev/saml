const access = document.querySelector('#access');
const accessForm = document.querySelector('#access-form');
const tokenInput = document.querySelector('#admin-token');
const dashboard = document.querySelector('#dashboard');
const fleet = document.querySelector('#fleet');
const message = document.querySelector('#message');
let token = '';
let authMode = 'token';

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

const updateNames = {
  installed: 'INSTALLED',
  rolled_back: 'ROLLED BACK',
  download_failed: 'DOWNLOAD FAILED',
  install_failed: 'INSTALL FAILED',
  rollback_failed: 'RESTORE FAILED',
};

const updateReasons = {
  agent_exited: 'New agent exited during startup',
  health_timeout: 'New agent did not pass its health check',
  state_save_failed: 'Could not save the new active version',
  target_fetch_failed: 'Could not download the release',
  target_invalid: 'Downloaded release failed verification',
  target_stage_failed: 'Could not stage the release locally',
  agent_stop_failed: 'Could not stop the previous agent',
  restore_failed: 'Could not restart the previous agent',
};

function renderUpdates(updates) {
  const section = element('div', 'agent-updates');
  section.append(element('div', 'update-heading', `UPDATE ACTIVITY / ${updates.length}`));
  if (!updates.length) {
    section.append(element('p', 'update-empty', 'No update attempts reported'));
    return section;
  }
  for (const event of updates.slice(-3).reverse()) {
    const row = element('div', `update-event ${event.outcome === 'installed' ? 'success' : 'failure'}`);
    const result = element('strong', 'update-result', updateNames[event.outcome] || event.outcome);
    const versions = element('span', 'update-versions', `v${event.from_version} → v${event.target_version}`);
    const detail = element('span', 'update-detail', updateReasons[event.reason] || 'Update completed');
    const time = element('time', 'update-time', new Date(event.at_unix_secs * 1000).toLocaleString());
    time.dateTime = new Date(event.at_unix_secs * 1000).toISOString();
    row.append(result, versions, detail, time);
    section.append(row);
  }
  return section;
}

function detailList(title, values, format = value => value) {
  const section = element('section', 'repo-detail-section');
  section.append(element('h4', '', title));
  if (!values.length) {
    section.append(element('p', 'repo-detail-empty', 'None reported'));
    return section;
  }
  const list = element('ul', 'repo-detail-list');
  for (const value of values) list.append(element('li', '', format(value)));
  section.append(list);
  return section;
}

function renderRepository(repository, agentId, openRepos) {
  const details = repository.details;
  const row = element('details', 'repo');
  row.dataset.key = `${agentId}:${repository.id}`;
  row.open = openRepos.has(row.dataset.key);
  const summary = element('summary', 'repo-summary');
  const heading = element('span', 'repo-heading');
  heading.append(element('strong', 'repo-name', details?.name || repository.id.slice(0, 18)));
  heading.append(element('span', 'repo-subtitle', details
    ? `${details.git.branch || 'detached / no Git'} · ${details.model.concept_count} concepts · ${details.model.materialization_count} items`
    : repository.id));
  summary.append(heading, badge(repository.health));
  row.append(summary);
  if (!details) {
    row.append(element('p', 'repo-detail-empty', 'Details are unavailable in this report.'));
    return row;
  }

  const body = element('div', 'repo-detail-body');
  const path = element('div', 'repo-paths');
  path.append(element('span', '', `SOURCE  ${details.path}`));
  path.append(element('span', '', `MODEL   ${details.model_root}`));
  body.append(path);

  const model = details.model;
  const git = details.git;
  const metrics = element('div', 'repo-facts');
  for (const [label, value] of [
    ['SOURCE FILES', model.source_file_count], ['CONCEPTS', model.concept_count],
    ['ITEMS', model.materialization_count], ['LINKS', model.link_count],
    ['BINDINGS', model.binding_count], ['ERRORS', model.errors],
    ['WARNINGS', model.warnings], ['GIT CHANGES', git.change_count],
  ]) {
    const fact = element('div', 'repo-fact');
    fact.append(element('strong', '', String(value)), element('span', '', label));
    metrics.append(fact);
  }
  body.append(metrics);
  if (!model.available) body.append(element('p', 'repo-detail-empty', 'Model workspace is unavailable.'));

  const grid = element('div', 'repo-detail-grid');
  grid.append(detailList('Concepts', model.concepts));
  grid.append(detailList('Model items', model.materializations, item => `${item.kind} ${item.name} · ${item.id}`));
  grid.append(detailList('Model sources', model.source_uris));
  grid.append(detailList('Diagnostics', model.diagnostics, item => `${item.severity.toUpperCase()} ${item.code || 'syntax'} · ${item.message} · ${item.uri}`));
  if (git.available) {
    grid.append(detailList('Branches', git.branches));
    grid.append(detailList('Recent commits', git.recent_commits, item => `${item.id.slice(0, 9)} · ${item.subject} · ${item.authored_at}`));
    grid.append(detailList('Working tree', git.changes, item => `${item.status} ${item.path}`));
  } else {
    grid.append(detailList('Git', ['No Git worktree detected']));
  }
  body.append(grid);
  if (model.truncated || git.truncated) {
    body.append(element('p', 'repo-truncated', 'Some lists are shortened for the dashboard. Counts reflect the full repository.'));
  }
  row.append(body);
  return row;
}

function renderAgent(id, record, now, updates, openRepos, expandedAgents) {
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
  repos.dataset.agent = id;
  repos.dataset.expanded = expandedAgents.has(id) ? 'true' : 'false';
  repos.append(element('div', 'repo-title', `REPOSITORIES / ${heartbeat.repositories.length}`));
  const visible = expandedAgents.has(id) ? heartbeat.repositories : heartbeat.repositories.slice(0, 30);
  for (const repository of visible) {
    repos.append(renderRepository(repository, id, openRepos));
  }
  if (!expandedAgents.has(id) && heartbeat.repositories.length > 30) {
    const more = element('button', 'repo-more', `SHOW ${heartbeat.repositories.length - 30} MORE REPOSITORIES`);
    more.type = 'button';
    more.addEventListener('click', () => {
      for (const repository of heartbeat.repositories.slice(30)) {
        repos.insertBefore(renderRepository(repository, id, openRepos), more);
      }
      repos.dataset.expanded = 'true';
      more.remove();
    });
    repos.append(more);
  }
  if (!heartbeat.repositories.length) {
    repos.append(element('p', 'repo-count', 'No repositories registered'));
  }
  card.append(main, repos, renderUpdates(updates));
  return card;
}

function render(data) {
  const openRepos = new Set([...fleet.querySelectorAll('details.repo[open]')].map(item => item.dataset.key));
  const expandedAgents = new Set([...fleet.querySelectorAll('.agent-repos[data-expanded="true"]')].map(item => item.dataset.agent));
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
  fleet.replaceChildren(...entries.map(([id, record]) =>
    renderAgent(id, record, now, data.update_events?.[id] || [], openRepos, expandedAgents)));
  document.querySelector('#empty').hidden = entries.length > 0;
}

async function load() {
  if (authMode === 'token' && !token) return;
  try {
    const response = await fetch('/api/v1/agents', {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      cache: 'no-store',
    });
    if (response.status === 401) {
      token = '';
      dashboard.hidden = true;
      if (authMode === 'token') {
        access.hidden = false;
        throw new Error('That administrator token was not accepted.');
      }
      throw new Error('Cloudflare access expired. Sign in again and reload.');
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

async function initialize() {
  try {
    const response = await fetch('/api/v1/config', { cache: 'no-store' });
    if (!response.ok) throw new Error(`Hub configuration failed (${response.status}).`);
    const config = await response.json();
    if (!['token', 'external'].includes(config.auth_mode)) {
      throw new Error('Unknown hub authentication mode.');
    }
    authMode = config.auth_mode;
    if (authMode === 'external') {
      document.querySelector('#disconnect').hidden = true;
      document.querySelector('#signal-index').textContent = 'LIVE SIGNAL / 01';
      document.querySelector('#registry-index').textContent = 'AGENT REGISTRY / 02';
      await load();
    } else {
      access.hidden = false;
    }
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
initialize();

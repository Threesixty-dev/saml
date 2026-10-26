const access = document.querySelector('#access');
const accessForm = document.querySelector('#access-form');
const tokenInput = document.querySelector('#admin-token');
const dashboard = document.querySelector('#dashboard');
const fleet = document.querySelector('#fleet');
const message = document.querySelector('#message');
const repositoryPage = document.querySelector('#repository-page');
const graphPage = document.querySelector('#graph-page');
const graphPanel = document.querySelector('#graph-panel');
let token = '';
let authMode = 'token';
let latestData = null;
let detailKey = '';
let graphLoaded = false;
let graphLoadingKey = '';

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

function repositoryUrl(agentId, repositoryId) {
  return `/repository?agent=${encodeURIComponent(agentId)}&repo=${encodeURIComponent(repositoryId)}`;
}

function graphUrl(agentId, repositoryId) {
  return `/graph?agent=${encodeURIComponent(agentId)}&repo=${encodeURIComponent(repositoryId)}`;
}

function renderRepository(repository, agentId) {
  const details = repository.details;
  const row = element('a', 'repo-row');
  row.href = repositoryUrl(agentId, repository.id);
  const heading = element('span', 'repo-heading');
  heading.append(element('strong', 'repo-name', details?.name || repository.id.slice(0, 18)));
  heading.append(element('span', 'repo-subtitle', details
    ? `${details.git.branch || 'detached / no Git'} · ${details.model.concept_count} concepts · ${details.model.materialization_count} items`
    : repository.id));
  row.append(heading, badge(repository.health), element('span', 'repo-arrow', '↗'));
  row.addEventListener('click', event => {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    history.pushState(null, '', row.href);
    showRoute();
  });
  return row;
}

function renderAgent(id, record, now, updates, expandedAgents) {
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
    repos.append(renderRepository(repository, id));
  }
  if (!expandedAgents.has(id) && heartbeat.repositories.length > 30) {
    const more = element('button', 'repo-more', `SHOW ${heartbeat.repositories.length - 30} MORE REPOSITORIES`);
    more.type = 'button';
    more.addEventListener('click', () => {
      for (const repository of heartbeat.repositories.slice(30)) {
        repos.insertBefore(renderRepository(repository, id), more);
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

function sourcePath(uri, details) {
  if (!uri) return 'unknown source';
  if (!uri.startsWith('file:')) return uri;
  try {
    const absolute = decodeURIComponent(new URL(uri).pathname);
    const repositoryRoot = details.path || '';
    const modelRoot = details.model_root || '';
    const inRepository = modelRoot === repositoryRoot || modelRoot.startsWith(`${repositoryRoot}/`);
    const base = inRepository ? repositoryRoot : modelRoot;
    if (base && absolute.startsWith(`${base}/`)) return absolute.slice(base.length + 1);
    return absolute.split('/').pop() || 'unknown source';
  } catch {
    return 'unknown source';
  }
}

function modelSources(details) {
  if (details.model.source_paths?.length) return details.model.source_paths;
  return (details.model.source_uris || []).map(uri => sourcePath(uri, details));
}

function modelLocation(details) {
  if (details.model.path_base === 'model_root' || details.model_root === 'external') {
    return 'External model · file paths are relative to its model root';
  }
  if (details.model.path_base === 'repository') return 'File paths are relative to the repository';
  const inside = details.model_root === details.path || details.model_root.startsWith(`${details.path}/`);
  return inside ? 'File paths are relative to the repository' : 'External model · file paths are relative to its model root';
}

async function loadGraph(agentId, repositoryId, key) {
  if (graphLoadingKey === key) return;
  graphLoadingKey = key;
  const empty = document.querySelector('#graph-empty');
  const caption = document.querySelector('#graph-caption');
  const svg = document.querySelector('#model-graph');
  const inspector = document.querySelector('#graph-inspector');
  empty.hidden = false;
  empty.textContent = 'Loading the model graph…';
  svg.replaceChildren();
  inspector.replaceChildren(element('span', 'section-index', 'INSPECTOR'), element('p', '', 'Choose a node to inspect its connections.'));
  try {
    const response = await fetch(`/api/v1/repository-graphs/${encodeURIComponent(agentId)}/${encodeURIComponent(repositoryId)}`, {
      headers: token ? { Authorization: `Bearer ${token}` } : {},
      cache: 'no-store',
    });
    if (key !== detailKey) return;
    if (response.status === 404) {
      empty.textContent = 'The graph has not arrived yet. The agent will send it with its next report.';
      caption.textContent = '';
      return;
    }
    if (!response.ok) throw new Error(`Graph request failed (${response.status}).`);
    const graph = await response.json();
    if (key !== detailKey) return;
    window.renderModelGraph(graph);
    graphLoaded = true;
    const items = document.querySelector('#detail-model-items');
    if (items && graph.available) {
      const fullList = detailList('Model items', graph.nodes, item => `${item.kind} ${item.name} · ${item.id}`);
      fullList.id = 'detail-model-items';
      if (graph.truncated) fullList.append(element('p', 'repo-truncated', `Showing ${graph.nodes.length} of ${graph.node_count} items.`));
      items.replaceWith(fullList);
    }
    caption.textContent = graph.truncated
      ? `Showing ${graph.nodes.length} of ${graph.node_count} items and ${graph.links.length} of ${graph.link_count} links.`
      : `${graph.node_count} items · ${graph.link_count} resolved links. Scroll to zoom; drag to pan.`;
  } catch (error) {
    if (key !== detailKey) return;
    empty.hidden = false;
    empty.textContent = error.message;
    caption.textContent = '';
  } finally {
    if (graphLoadingKey === key) graphLoadingKey = '';
  }
}

function renderDetail(data, fullscreen = false) {
  const params = new URLSearchParams(location.search);
  const agentId = params.get('agent');
  const repositoryId = params.get('repo');
  const record = data.agents?.[agentId];
  const repository = record?.heartbeat.repositories.find(item => item.id === repositoryId);
  document.querySelector(fullscreen ? '#graph-page-slot' : '#repository-graph-slot').append(graphPanel);
  document.querySelector('#fleet-hero').hidden = true;
  dashboard.hidden = true;
  repositoryPage.hidden = fullscreen;
  graphPage.hidden = !fullscreen;
  document.querySelector('#open-graph').href = graphUrl(agentId, repositoryId);
  document.querySelector('#back-to-repository').href = repositoryUrl(agentId, repositoryId);
  if (!repository) {
    detailKey = '';
    graphLoaded = false;
    document.title = 'Repository unavailable / SAML Observatory';
    document.querySelector('#detail-name').textContent = 'Repository unavailable';
    document.querySelector('#detail-subtitle').textContent = 'This repository is not in the latest fleet report.';
    document.querySelector('#graph-repository-name').textContent = 'Repository unavailable';
    document.querySelector('#graph-repository-meta').textContent = 'No current report';
    document.querySelector('#detail-facts').replaceChildren();
    document.querySelector('#detail-sections').replaceChildren();
    document.querySelector('#model-graph').replaceChildren();
    document.querySelector('#graph-search').value = '';
    document.querySelector('#graph-inspector').replaceChildren(
      element('span', 'section-index', 'INSPECTOR'),
      element('p', '', 'No graph items to inspect.'),
    );
    document.querySelector('#graph-empty').textContent = 'No graph report available.';
    document.querySelector('#graph-empty').hidden = false;
    document.querySelector('#graph-caption').textContent = '';
    return;
  }
  const details = repository.details;
  const key = `${agentId}:${repositoryId}`;
  const name = details?.name || repositoryId.slice(0, 18);
  document.title = `${name}${fullscreen ? ' graph' : ''} / SAML Observatory`;
  document.querySelector('#detail-name').textContent = name;
  document.querySelector('#graph-repository-name').textContent = name;
  document.querySelector('#graph-repository-meta').textContent = `${details?.git.branch || 'Detached / no Git'} · Agent ${agentId.slice(0, 8)}`;
  document.querySelector('#detail-subtitle').textContent = details
    ? `${details.git.branch || 'Detached / no Git'} · ${modelLocation(details)}`
    : 'Repository details are not available in this heartbeat.';
  const identity = document.querySelector('.detail-identity');
  identity.replaceChildren(
    badge(repository.health),
    element('span', '', `AGENT ${agentId.slice(0, 8)} · v${record.heartbeat.version}`),
    element('span', '', details?.git.head ? `HEAD ${details.git.head.slice(0, 12)}` : 'NO GIT HEAD'),
  );
  if (key === detailKey) {
    if (!graphLoaded) loadGraph(agentId, repositoryId, key);
    return;
  }
  detailKey = key;
  graphLoaded = false;
  const facts = document.querySelector('#detail-facts');
  const sections = document.querySelector('#detail-sections');
  facts.replaceChildren();
  sections.replaceChildren();
  if (!details) {
    sections.append(element('p', 'repo-detail-empty', 'The agent omitted details to keep its heartbeat within the size limit.'));
    loadGraph(agentId, repositoryId, key);
    return;
  }
  const model = details.model;
  const git = details.git;
  for (const [label, value] of [
    ['SOURCE FILES', model.source_file_count], ['CONCEPTS', model.concept_count],
    ['MODEL ITEMS', model.materialization_count], ['LINKS', model.link_count],
    ['BINDINGS', model.binding_count], ['ERRORS', model.errors],
    ['WARNINGS', model.warnings], ['GIT CHANGES', git.change_count],
  ]) {
    const fact = element('div', 'detail-fact');
    fact.append(element('strong', '', String(value)), element('span', '', label));
    facts.append(fact);
  }
  const sources = detailList('Model files', modelSources(details));
  sources.querySelector('h4').after(element('p', 'source-note', modelLocation(details)));
  sections.append(sources);
  sections.append(detailList(model.concept_count > model.concepts.length ? 'Concepts · preview' : 'Concepts', model.concepts));
  const items = detailList('Model items · preview', model.materializations, item => `${item.kind} ${item.name} · ${item.id}`);
  items.id = 'detail-model-items';
  sections.append(items);
  sections.append(detailList('Diagnostics', model.diagnostics, item =>
    `${item.severity.toUpperCase()} ${item.code || 'syntax'} · ${item.message} · ${sourcePath(item.uri, details)}`));
  if (git.available) {
    sections.append(detailList('Branches', git.branches));
    sections.append(detailList('Recent commits', git.recent_commits, item => `${item.id.slice(0, 9)} · ${item.subject} · ${item.authored_at}`));
    sections.append(detailList('Working tree', git.changes, item => `${item.status} ${item.path}`));
  } else {
    sections.append(detailList('Git', ['No Git worktree detected']));
  }
  if (model.truncated || git.truncated) {
    sections.append(element('p', 'repo-truncated', 'List previews are bounded. Counts reflect the full repository; the graph has its own coverage indicator.'));
  }
  loadGraph(agentId, repositoryId, key);
}

function showRoute() {
  if (latestData) render(latestData);
  else load();
  window.scrollTo({ top: 0, behavior: 'instant' });
}

function render(data) {
  latestData = data;
  const fullscreen = location.pathname === '/graph';
  document.body.classList.toggle('graph-mode', fullscreen);
  if (location.pathname === '/repository' || fullscreen) {
    renderDetail(data, fullscreen);
    return;
  }
  document.title = 'SAML / Observatory';
  detailKey = '';
  graphLoaded = false;
  document.querySelector('#fleet-hero').hidden = false;
  repositoryPage.hidden = true;
  graphPage.hidden = true;
  dashboard.hidden = false;
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
    renderAgent(id, record, now, data.update_events?.[id] || [], expandedAgents)));
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
document.querySelector('#refresh-detail').addEventListener('click', () => {
  detailKey = '';
  graphLoaded = false;
  load();
});
document.querySelector('#disconnect').addEventListener('click', () => {
  token = '';
  dashboard.hidden = true;
  repositoryPage.hidden = true;
  graphPage.hidden = true;
  access.hidden = false;
  message.textContent = '';
  tokenInput.focus();
});
document.querySelector('#back-to-fleet').addEventListener('click', event => {
  event.preventDefault();
  history.pushState(null, '', '/');
  showRoute();
});
for (const id of ['#open-graph', '#back-to-repository']) {
  document.querySelector(id).addEventListener('click', event => {
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    event.preventDefault();
    history.pushState(null, '', event.currentTarget.href);
    showRoute();
  });
}
window.addEventListener('popstate', showRoute);

function updateClock() {
  document.querySelector('#clock').textContent = new Date().toLocaleTimeString([], {
    hour: '2-digit', minute: '2-digit',
  });
}
updateClock();
setInterval(updateClock, 30_000);
setInterval(load, 10_000);
initialize();

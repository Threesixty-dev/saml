const graphNamespace = 'http://www.w3.org/2000/svg';

function graphElement(tag, attributes = {}, text = '') {
  const node = document.createElementNS(graphNamespace, tag);
  for (const [name, value] of Object.entries(attributes)) node.setAttribute(name, value);
  if (text) node.textContent = text;
  return node;
}

function graphText(tag, className, value) {
  const node = document.createElement(tag);
  node.className = className;
  node.textContent = value;
  return node;
}

window.renderModelGraph = function renderModelGraph(graph) {
  const svg = document.querySelector('#model-graph');
  const empty = document.querySelector('#graph-empty');
  const inspector = document.querySelector('#graph-inspector');
  const search = document.querySelector('#graph-search');
  svg.replaceChildren();
  svg.classList.remove('searching');
  search.value = '';
  if (!graph.available || !graph.nodes?.length) {
    empty.hidden = false;
    empty.textContent = graph.available ? 'No model items were found.' : 'This model workspace is unavailable.';
    inspector.replaceChildren(graphText('p', '', 'No graph items to inspect.'));
    return;
  }
  empty.hidden = true;

  const nodes = [...graph.nodes].sort((a, b) => a.kind.localeCompare(b.kind) || a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
  const byKind = new Map();
  for (const node of nodes) {
    if (!byKind.has(node.kind)) byKind.set(node.kind, []);
    byKind.get(node.kind).push(node);
  }
  const kinds = [...byKind.keys()].sort();
  const columns = Math.max(1, Math.ceil(Math.sqrt(kinds.length * 1.5)));
  const rows = Math.ceil(kinds.length / columns);
  let maxRing = 0;
  for (const group of byKind.values()) {
    let remaining = group.length - 1;
    let ring = 0;
    while (remaining > 0) {
      ring++;
      remaining -= ring * 8;
    }
    maxRing = Math.max(maxRing, ring);
  }
  const cellWidth = Math.max(340, maxRing * 50 + 90);
  const cellHeight = Math.max(300, maxRing * 50 + 100);
  const width = columns * cellWidth;
  const height = rows * cellHeight;
  const palette = ['#2e7669', '#bc7151', '#b99752', '#608d9a', '#758b62', '#976c89'];
  const positions = new Map();
  const background = graphElement('g', { class: 'graph-groups' });
  const edgeLayer = graphElement('g', { class: 'graph-edges' });
  const nodeLayer = graphElement('g', { class: 'graph-nodes' });
  const nodeElements = new Map();
  const edgeElements = [];

  kinds.forEach((kind, groupIndex) => {
    const col = groupIndex % columns;
    const row = Math.floor(groupIndex / columns);
    const left = col * cellWidth;
    const top = row * cellHeight;
    const color = palette[groupIndex % palette.length];
    background.append(graphElement('rect', {
      x: left + 10, y: top + 10, width: cellWidth - 20, height: cellHeight - 20,
      rx: 16, class: 'graph-group-box',
    }));
    background.append(graphElement('text', {
      x: left + 30, y: top + 39, class: 'graph-group-label',
    }, `${kind.toUpperCase()} / ${byKind.get(kind).length}`));
    byKind.get(kind).forEach((node, index) => {
      let x = left + cellWidth / 2;
      let y = top + cellHeight / 2 + 12;
      if (index > 0) {
        let remaining = index - 1;
        let ring = 1;
        while (remaining >= ring * 8) {
          remaining -= ring * 8;
          ring++;
        }
        const angle = 2 * Math.PI * remaining / (ring * 8) - Math.PI / 2;
        x += Math.cos(angle) * ring * 25;
        y += Math.sin(angle) * ring * 25;
      }
      positions.set(node.id, { x, y, color, node });
    });
  });

  for (const link of graph.links || []) {
    const from = positions.get(link.from);
    const to = positions.get(link.to);
    if (!from || !to) continue;
    const edge = graphElement('line', {
      x1: from.x, y1: from.y, x2: to.x, y2: to.y,
      class: 'graph-edge', 'data-from': link.from, 'data-to': link.to,
    });
    edge.append(graphElement('title', {}, `${from.node.name} → ${to.node.name} · ${link.relation}`));
    edgeLayer.append(edge);
    edgeElements.push({ element: edge, link });
  }
  for (const item of positions.values()) {
    const circle = graphElement('circle', {
      cx: item.x, cy: item.y, r: 8, fill: item.color,
      class: 'graph-node', 'data-node-id': item.node.id,
      tabindex: '0', role: 'button',
      'aria-label': `${item.node.kind} ${item.node.name}`,
    });
    circle.append(graphElement('title', {}, `${item.node.kind} ${item.node.name}`));
    circle.addEventListener('click', () => select(item.node.id));
    circle.addEventListener('keydown', event => {
      if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        select(item.node.id);
      }
    });
    nodeLayer.append(circle);
    if (byKind.get(item.node.kind).length <= 12) {
      const groupIndex = kinds.indexOf(item.node.kind);
      const groupCenter = (groupIndex % columns + .5) * cellWidth;
      const toLeft = item.x > groupCenter + 10;
      nodeLayer.append(graphElement('text', {
        x: item.x + (toLeft ? -13 : 13), y: item.y + 4,
        'text-anchor': toLeft ? 'end' : 'start', class: 'graph-node-label',
      }, item.node.name.length > 18 ? `${item.node.name.slice(0, 17)}…` : item.node.name));
    }
    nodeElements.set(item.node.id, circle);
  }
  svg.append(background, edgeLayer, nodeLayer);

  let view = { x: 0, y: 0, w: width, h: height };
  let selectedId = null;
  let pointer = null;
  function updateView() {
    svg.setAttribute('viewBox', `${view.x} ${view.y} ${view.w} ${view.h}`);
  }
  function zoom(factor, clientX, clientY) {
    const rect = svg.getBoundingClientRect();
    const fx = rect.width ? (clientX - rect.left) / rect.width : .5;
    const fy = rect.height ? (clientY - rect.top) / rect.height : .5;
    const nextW = Math.max(130, Math.min(width * 1.5, view.w / factor));
    const nextH = nextW * view.h / view.w;
    view = {
      x: view.x + fx * (view.w - nextW),
      y: view.y + fy * (view.h - nextH),
      w: nextW, h: nextH,
    };
    updateView();
  }
  function centerOn(id) {
    const item = positions.get(id);
    if (!item) return;
    view = { x: item.x - 180, y: item.y - 150, w: 360, h: 300 };
    updateView();
  }
  function select(id) {
    selectedId = id;
    const item = positions.get(id);
    if (!item) return;
    const related = edgeElements.filter(({ link }) => link.from === id || link.to === id);
    const neighbors = new Set(related.flatMap(({ link }) => [link.from, link.to]));
    for (const [nodeId, circle] of nodeElements) {
      circle.classList.toggle('selected', nodeId === id);
      circle.classList.toggle('connected', neighbors.has(nodeId) && nodeId !== id);
      circle.classList.toggle('dimmed', nodeId !== id && !neighbors.has(nodeId));
    }
    for (const { element, link } of edgeElements) {
      const connected = link.from === id || link.to === id;
      element.classList.toggle('selected', connected);
      element.classList.toggle('dimmed', !connected);
    }
    inspector.replaceChildren(
      graphText('span', 'section-index', 'SELECTED ITEM'),
      graphText('h3', '', item.node.name),
      graphText('p', 'inspector-kind', `${item.node.kind} · ${item.node.id}`),
      graphText('span', 'inspector-count', `${related.length} DIRECT CONNECTIONS`),
    );
    const list = graphText('div', 'inspector-links', '');
    for (const { link } of related.slice(0, 60)) {
      const outgoing = link.from === id;
      const other = positions.get(outgoing ? link.to : link.from);
      if (!other) continue;
      const button = graphText('button', 'inspector-link', `${outgoing ? '→' : '←'} ${link.relation} · ${other.node.name}`);
      button.type = 'button';
      button.addEventListener('click', () => { select(other.node.id); centerOn(other.node.id); });
      list.append(button);
    }
    if (related.length > 60) list.append(graphText('p', 'inspector-overflow', `${related.length - 60} more connections`));
    inspector.append(list);
  }
  function reset() {
    view = { x: 0, y: 0, w: width, h: height };
    selectedId = null;
    search.value = '';
    svg.classList.remove('searching');
    for (const circle of nodeElements.values()) circle.classList.remove('selected', 'connected', 'dimmed', 'match');
    for (const { element } of edgeElements) element.classList.remove('selected', 'dimmed');
    inspector.replaceChildren(graphText('span', 'section-index', 'INSPECTOR'), graphText('p', '', 'Choose a node to inspect its connections.'));
    updateView();
  }
  document.querySelector('#graph-zoom-in').onclick = () => zoom(1.3, svg.getBoundingClientRect().left + svg.clientWidth / 2, svg.getBoundingClientRect().top + svg.clientHeight / 2);
  document.querySelector('#graph-zoom-out').onclick = () => zoom(1 / 1.3, svg.getBoundingClientRect().left + svg.clientWidth / 2, svg.getBoundingClientRect().top + svg.clientHeight / 2);
  document.querySelector('#graph-reset').onclick = reset;
  search.oninput = () => {
    const query = search.value.trim().toLowerCase();
    svg.classList.toggle('searching', Boolean(query));
    let first = null;
    for (const [id, circle] of nodeElements) {
      const node = positions.get(id).node;
      const match = query && `${node.name} ${node.kind} ${node.id}`.toLowerCase().includes(query);
      circle.classList.toggle('match', Boolean(match));
      if (match && !first) first = id;
    }
    if (first) centerOn(first);
  };
  search.onkeydown = event => {
    if (event.key !== 'Enter') return;
    const first = [...nodeElements].find(([, circle]) => circle.classList.contains('match'));
    if (first) select(first[0]);
  };
  svg.onwheel = event => {
    event.preventDefault();
    zoom(event.deltaY < 0 ? 1.18 : 1 / 1.18, event.clientX, event.clientY);
  };
  svg.onpointerdown = event => {
    if (event.target.closest('[data-node-id]')) return;
    pointer = { x: event.clientX, y: event.clientY, viewX: view.x, viewY: view.y };
    svg.setPointerCapture(event.pointerId);
  };
  svg.onpointermove = event => {
    if (!pointer) return;
    const rect = svg.getBoundingClientRect();
    view.x = pointer.viewX - (event.clientX - pointer.x) * view.w / rect.width;
    view.y = pointer.viewY - (event.clientY - pointer.y) * view.h / rect.height;
    updateView();
  };
  svg.onpointerup = () => { pointer = null; };
  svg.onpointercancel = () => { pointer = null; };
  reset();
};

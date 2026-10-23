const canvas = document.querySelector('#graph');
const edgeCanvas = document.querySelector('#edge-overlay');
const edgeContext = edgeCanvas.getContext('2d');
const stage = document.querySelector('#stage');
const labelsLayer = document.querySelector('#node-labels');
const tooltip = document.querySelector('#tooltip');
const inspector = document.querySelector('#inspector');
const search = document.querySelector('#search');
const results = document.querySelector('#search-results');
const selection = document.querySelector('#selection');
const loading = document.querySelector('#loading');
const errorBox = document.querySelector('#graph-error');

const colors = {
  concept: [0.54, 0.95, 0.79],
  materialization: [0.44, 0.79, 0.83],
  boundary: [0.94, 0.72, 0.42],
  unknown: [0.94, 0.52, 0.44],
  extends: [0.96, 0.56, 0.47],
};
const filters = { concept: true, materialization: true, boundary: true, unknown: true };
const camera = { yaw: 0.49, pitch: 0.2, distance: 26, target: [0, 0, 0] };
const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

let gl;
let program;
let lineGeometry;
let pointGeometry;
let uniforms;
let map;
let nodes = [];
let edges = [];
let selected = null;
let hovered = null;
let spinning = !reducedMotion;
let pointer = null;
let lastFrame = 0;
let size = { width: 1, height: 1, dpr: 1 };
let labelElements = [];

function compileShader(type, source) {
  const shader = gl.createShader(type);
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    throw new Error(gl.getShaderInfoLog(shader) || 'Shader compilation failed');
  }
  return shader;
}

function initGl() {
  gl = canvas.getContext('webgl2', { alpha: true, antialias: true, premultipliedAlpha: false });
  if (!gl) throw new Error('This browser needs WebGL 2 to display the 3D atlas.');
  const vertex = compileShader(gl.VERTEX_SHADER, `#version 300 es
    precision highp float;
    layout(location=0) in vec3 a_position;
    layout(location=1) in vec3 a_color;
    layout(location=2) in float a_size;
    layout(location=3) in float a_alpha;
    uniform vec3 u_right;
    uniform vec3 u_up;
    uniform vec3 u_forward;
    uniform vec3 u_target;
    uniform float u_distance;
    uniform float u_aspect;
    uniform float u_focal;
    uniform float u_pixel_ratio;
    uniform float u_size_scale;
    out vec3 v_color;
    out float v_alpha;
    void main() {
      vec3 p = a_position - u_target;
      float depth = u_distance - dot(p, u_forward);
      float z = max(depth, 0.1);
      gl_Position = vec4(dot(p, u_right) * u_focal / u_aspect,
                         dot(p, u_up) * u_focal,
                         z * 1.002002 - 0.2002, z);
      gl_PointSize = clamp(a_size * u_pixel_ratio * u_size_scale * u_distance / z, 2.0, 100.0);
      v_color = a_color;
      v_alpha = a_alpha;
    }
  `);
  const fragment = compileShader(gl.FRAGMENT_SHADER, `#version 300 es
    precision highp float;
    in vec3 v_color;
    in float v_alpha;
    uniform float u_point_mode;
    uniform float u_glow;
    out vec4 out_color;
    void main() {
      float alpha = v_alpha;
      if (u_point_mode > 0.5) {
        float radius = length(gl_PointCoord * 2.0 - 1.0);
        if (radius > 1.0) discard;
        alpha *= u_glow > 0.5
          ? 0.29 * exp(-3.6 * radius * radius)
          : 1.0 - smoothstep(0.66, 1.0, radius);
      }
      out_color = vec4(v_color, alpha);
    }
  `);
  program = gl.createProgram();
  gl.attachShader(program, vertex);
  gl.attachShader(program, fragment);
  gl.linkProgram(program);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    throw new Error(gl.getProgramInfoLog(program) || 'Shader link failed');
  }
  gl.useProgram(program);
  uniforms = Object.fromEntries([
    'right', 'up', 'forward', 'target', 'distance', 'aspect', 'focal',
    'pixel_ratio', 'size_scale', 'point_mode', 'glow',
  ].map(name => [name, gl.getUniformLocation(program, `u_${name}`)]));
  lineGeometry = makeGeometry();
  pointGeometry = makeGeometry();
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
  gl.enable(gl.DEPTH_TEST);
  gl.depthFunc(gl.LEQUAL);
}

function makeGeometry() {
  const vao = gl.createVertexArray();
  const buffer = gl.createBuffer();
  gl.bindVertexArray(vao);
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  for (const [index, length, offset] of [[0, 3, 0], [1, 3, 12], [2, 1, 24], [3, 1, 28]]) {
    gl.enableVertexAttribArray(index);
    gl.vertexAttribPointer(index, length, gl.FLOAT, false, 32, offset);
  }
  gl.bindVertexArray(null);
  return { vao, buffer, count: 0 };
}

function upload(geometry, values) {
  gl.bindBuffer(gl.ARRAY_BUFFER, geometry.buffer);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(values), gl.DYNAMIC_DRAW);
  geometry.count = values.length / 8;
}

function hash(value) {
  let result = 2166136261;
  for (let i = 0; i < value.length; i++) {
    result = Math.imul(result ^ value.charCodeAt(i), 16777619);
  }
  return result >>> 0;
}

function jitter(key, salt) {
  return (hash(`${key}:${salt}`) / 4294967295) * 2 - 1;
}

function categoryFor(materialization, concepts) {
  const concept = concepts.get(materialization.kind);
  if (!concept) return 'unknown';
  return concept.roles.includes('boundary') ? 'boundary' : 'materialization';
}

function buildGraph(data) {
  map = data;
  const concepts = new Map(data.concepts.map(concept => [concept.word, concept]));
  const conceptNodes = data.concepts.map((concept, index) => {
    const angle = index * Math.PI * 2 / data.concepts.length - 0.4;
    return {
      id: `concept:${concept.word}`, name: concept.word, category: 'concept', data: concept,
      position: [Math.cos(angle) * 6.3, Math.sin(index * 1.9) * 2.5, Math.sin(angle) * 6.3],
      fixed: true,
    };
  });
  const conceptByKind = new Map(conceptNodes.map(node => [node.name, node]));
  const materializationNodes = data.materializations.map((materialization, index) => {
    const parent = conceptByKind.get(materialization.kind);
    const base = parent ? parent.position : [0, -5, 0];
    const direction = index * 2.399963;
    return {
      id: `materialization:${materialization.id}`, name: materialization.name,
      category: categoryFor(materialization, concepts), data: materialization,
      position: [
        base[0] * .75 + Math.cos(direction) * 2.7 + jitter(materialization.id, 0),
        base[1] + Math.sin(direction) * 2 + jitter(materialization.id, 1),
        base[2] * .75 + Math.sin(direction) * 2.7 + jitter(materialization.id, 2),
      ],
      fixed: false,
    };
  });
  nodes = [...conceptNodes, ...materializationNodes];
  const byId = new Map(nodes.map(node => [node.id, node]));
  edges = [];
  for (const materialization of materializationNodes) {
    const concept = conceptByKind.get(materialization.data.kind);
    if (concept) edges.push({ from: concept, to: materialization, relation: 'materializes' });
  }
  const edgeKeys = new Set();
  for (const link of data.links) {
    const from = byId.get(`materialization:${link.from}`);
    const to = byId.get(`materialization:${link.to}`);
    if (!from || !to) continue;
    const key = `${from.id}\u0000${to.id}\u0000${link.relation}`;
    if (edgeKeys.has(key)) continue;
    edgeKeys.add(key);
    edges.push({ from, to, relation: link.relation, crossesScope: link.crosses_scope, crossesBoundary: link.crosses_boundary });
  }
  relaxLayout();
  document.querySelector('#concept-count').textContent = data.concepts.length;
  document.querySelector('#materialization-count').textContent = data.materializations.length;
  document.querySelector('#link-count').textContent = data.links.length;
  document.querySelector('#finding-count').textContent = data.findings.length;
  renderFindings(data.findings);
  selected = selected && byId.get(selected.id) || null;
  if (selected) renderSelection();
  rebuildLabels();
  refreshSearch();
}

function relaxLayout() {
  const velocity = nodes.map(() => [0, 0, 0]);
  const index = new Map(nodes.map((node, i) => [node, i]));
  for (let step = 0; step < 210; step++) {
    const forces = nodes.map(() => [0, 0, 0]);
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        const a = nodes[i].position;
        const b = nodes[j].position;
        const delta = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
        const distance2 = Math.max(0.7, delta[0] ** 2 + delta[1] ** 2 + delta[2] ** 2);
        const strength = 0.13 / (distance2 * Math.sqrt(distance2));
        for (let axis = 0; axis < 3; axis++) {
          forces[i][axis] += delta[axis] * strength;
          forces[j][axis] -= delta[axis] * strength;
        }
      }
    }
    for (const edge of edges) {
      const a = edge.from.position;
      const b = edge.to.position;
      const delta = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
      const distance = Math.max(.1, Math.hypot(...delta));
      const desired = edge.relation === 'materializes' ? 2.8 : edge.relation === 'boundary' ? 3.8 : 4.6;
      const strength = (distance - desired) * (edge.relation === 'materializes' ? .012 : .006);
      const ai = index.get(edge.from);
      const bi = index.get(edge.to);
      for (let axis = 0; axis < 3; axis++) {
        const force = delta[axis] / distance * strength;
        forces[ai][axis] += force;
        forces[bi][axis] -= force;
      }
    }
    nodes.forEach((node, i) => {
      if (node.fixed) return;
      for (let axis = 0; axis < 3; axis++) {
        velocity[i][axis] = (velocity[i][axis] + forces[i][axis]) * .78;
        node.position[axis] += velocity[i][axis];
        node.position[axis] = Math.max(-11, Math.min(11, node.position[axis]));
      }
    });
  }
}

function visible(node) { return filters[node.category]; }

function emphasized(node) {
  const query = search.value.trim().toLowerCase();
  if (selected) {
    return node === selected || edges.some(edge =>
      (edge.from === selected && edge.to === node) || (edge.to === selected && edge.from === node));
  }
  return !query || node.name.toLowerCase().includes(query) || node.data.kind?.toLowerCase().includes(query);
}

function edgeColor(edge) {
  if (edge.relation === 'materializes') return colors.concept;
  if (edge.relation === 'boundary') return colors.boundary;
  if (edge.relation === 'extends') return colors.extends;
  return colors.materialization;
}

function vertex(values, position, color, pointSize, alpha) {
  values.push(...position, ...color, pointSize, alpha);
}

function updateGeometry() {
  const lines = [];
  const points = [];
  for (const edge of edges) {
    if (!visible(edge.from) || !visible(edge.to)) continue;
    const connected = !selected || edge.from === selected || edge.to === selected;
    const alpha = selected ? (connected ? .82 : .04) : edge.relation === 'materializes' ? .34 : .52;
    const color = edgeColor(edge);
    vertex(lines, edge.from.position, color, 1, alpha);
    vertex(lines, edge.to.position, color, 1, alpha);
  }
  for (const node of nodes) {
    if (!visible(node)) continue;
    const pointSize = node.category === 'concept' ? 17 : node.category === 'boundary' ? 13 : 9;
    const alpha = emphasized(node) ? 1 : .13;
    vertex(points, node.position, colors[node.category], node === selected ? pointSize * 1.5 : pointSize, alpha);
  }
  upload(lineGeometry, lines);
  upload(pointGeometry, points);
}

function basis() {
  const { yaw, pitch } = camera;
  return {
    forward: [Math.sin(yaw) * Math.cos(pitch), Math.sin(pitch), Math.cos(yaw) * Math.cos(pitch)],
    right: [Math.cos(yaw), 0, -Math.sin(yaw)],
    up: [-Math.sin(yaw) * Math.sin(pitch), Math.cos(pitch), -Math.cos(yaw) * Math.sin(pitch)],
  };
}

function focalLength() {
  return 1.76 * Math.min(1, size.width / size.height);
}

function project(position, vectors) {
  const p = position.map((value, axis) => value - camera.target[axis]);
  const dot = direction => p[0] * direction[0] + p[1] * direction[1] + p[2] * direction[2];
  const depth = camera.distance - dot(vectors.forward);
  if (depth < .5) return null;
  const focal = focalLength();
  return {
    x: size.width * .5 + dot(vectors.right) * focal / depth * size.height * .5,
    y: size.height * .5 - dot(vectors.up) * focal / depth * size.height * .5,
    depth,
  };
}

function resize() {
  const rect = stage.getBoundingClientRect();
  const dpr = Math.min(window.devicePixelRatio || 1, 2);
  size = { width: rect.width, height: rect.height, dpr };
  canvas.width = Math.round(rect.width * dpr);
  canvas.height = Math.round(rect.height * dpr);
  edgeCanvas.width = canvas.width;
  edgeCanvas.height = canvas.height;
  gl.viewport(0, 0, canvas.width, canvas.height);
}

function drawProjectedEdges(vectors) {
  edgeContext.setTransform(size.dpr, 0, 0, size.dpr, 0, 0);
  edgeContext.clearRect(0, 0, size.width, size.height);
  edgeContext.globalCompositeOperation = 'screen';
  for (const edge of edges) {
    if (!visible(edge.from) || !visible(edge.to)) continue;
    const from = project(edge.from.position, vectors);
    const to = project(edge.to.position, vectors);
    if (!from || !to) continue;
    const connected = !selected || edge.from === selected || edge.to === selected;
    const color = edgeColor(edge).map(value => Math.round(value * 255));
    const opacity = selected ? (connected ? .58 : .035) : edge.relation === 'materializes' ? .2 : .33;
    edgeContext.strokeStyle = `rgba(${color.join(',')},${opacity})`;
    edgeContext.lineWidth = selected && connected ? 1.6 : edge.relation === 'materializes' ? .85 : 1.1;
    edgeContext.beginPath();
    edgeContext.moveTo(from.x, from.y);
    edgeContext.lineTo(to.x, to.y);
    edgeContext.stroke();
  }
  edgeContext.globalCompositeOperation = 'source-over';
}

function draw(time) {
  const delta = Math.min(50, time - (lastFrame || time));
  lastFrame = time;
  if (spinning && !pointer) camera.yaw += delta * .00016;
  gl.viewport(0, 0, canvas.width, canvas.height);
  gl.clearColor(0, 0, 0, 0);
  gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
  gl.useProgram(program);
  const vectors = basis();
  gl.uniform3fv(uniforms.right, vectors.right);
  gl.uniform3fv(uniforms.up, vectors.up);
  gl.uniform3fv(uniforms.forward, vectors.forward);
  gl.uniform3fv(uniforms.target, camera.target);
  gl.uniform1f(uniforms.distance, camera.distance);
  gl.uniform1f(uniforms.aspect, size.width / size.height);
  gl.uniform1f(uniforms.focal, focalLength());
  gl.uniform1f(uniforms.pixel_ratio, size.dpr);

  gl.depthMask(false);
  gl.bindVertexArray(lineGeometry.vao);
  gl.uniform1f(uniforms.point_mode, 0);
  gl.uniform1f(uniforms.glow, 0);
  gl.uniform1f(uniforms.size_scale, 1);
  gl.drawArrays(gl.LINES, 0, lineGeometry.count);

  gl.bindVertexArray(pointGeometry.vao);
  gl.uniform1f(uniforms.point_mode, 1);
  gl.uniform1f(uniforms.glow, 1);
  gl.uniform1f(uniforms.size_scale, 2.9);
  gl.drawArrays(gl.POINTS, 0, pointGeometry.count);
  gl.depthMask(true);
  gl.uniform1f(uniforms.glow, 0);
  gl.uniform1f(uniforms.size_scale, 1);
  gl.drawArrays(gl.POINTS, 0, pointGeometry.count);
  gl.bindVertexArray(null);

  drawProjectedEdges(vectors);
  updateLabels(vectors);
  requestAnimationFrame(draw);
}

function rebuildLabels() {
  labelsLayer.replaceChildren();
  labelElements = [];
  for (const node of nodes) {
    if (node.category !== 'concept' && node.category !== 'boundary') continue;
    const label = document.createElement('div');
    label.className = `node-label ${node.category}`;
    label.textContent = node.name;
    labelsLayer.append(label);
    labelElements.push({ node, label });
  }
}

function updateLabels(vectors) {
  for (const { node, label } of labelElements) {
    const at = project(node.position, vectors);
    const compact = size.width < 600;
    const primary = node.category === 'concept' && ['entity', 'action', 'scope'].includes(node.name);
    const display = visible(node) && at && at.x > 0 && at.x < size.width && at.y > 130 &&
      at.y < size.height - 70 && (!compact || primary || node === selected || node === hovered);
    label.hidden = !display;
    if (!display) continue;
    label.style.left = `${at.x}px`;
    label.style.top = `${at.y}px`;
    label.style.transform = at.x > size.width - 165 ? 'translate(calc(-100% - 13px), -50%)' : 'translate(13px, -50%)';
    label.style.opacity = emphasized(node) ? Math.max(.47, Math.min(1, 28 / at.depth)) : .16;
    label.classList.toggle('selected', node === selected);
  }
}

function pick(x, y) {
  const vectors = basis();
  let closest = null;
  let score = Infinity;
  for (const node of nodes) {
    if (!visible(node)) continue;
    const at = project(node.position, vectors);
    if (!at) continue;
    const distance = Math.hypot(x - at.x, y - at.y);
    const radius = node.category === 'concept' ? 18 : 14;
    if (distance < radius && distance + at.depth * .03 < score) {
      closest = node;
      score = distance + at.depth * .03;
    }
  }
  return closest;
}

function fileName(uri) {
  try { return decodeURIComponent(uri.split('/').pop()); }
  catch { return uri.split('/').pop(); }
}

function element(tag, className, text) {
  const item = document.createElement(tag);
  if (className) item.className = className;
  if (text !== undefined) item.textContent = text;
  return item;
}

function detailLabel(card, title) { card.append(element('span', 'detail-label', title)); }

function renderSelection() {
  selection.replaceChildren();
  if (!selected) {
    selection.className = 'empty-selection';
    selection.append(
      element('div', 'empty-glyph', '✳'),
      element('strong', '', 'Choose a node'),
      element('span', '', 'Click a point in space or search by name to trace its meaning.'),
    );
    return;
  }
  selection.className = `selection-card ${selected.category}`;
  const data = selected.data;
  const role = selected.category === 'boundary' ? 'boundary materialization' :
    selected.category === 'unknown' ? 'undeclared materialization' : 'materialization';
  selection.append(element('span', 'type', selected.category === 'concept' ? 'SPEC CONCEPT' : `${data.kind} / ${role}`));
  selection.append(element('h3', '', selected.name));
  if (selected.category === 'concept') {
    detailLabel(selection, 'ROLES');
    const chips = element('div', 'chips');
    for (const role of data.roles) chips.append(element('span', 'chip', role));
    selection.append(chips);
    detailLabel(selection, 'MATERIALIZATIONS CREATED');
    const count = nodes.filter(node => node.category !== 'concept' && node.data.kind === data.word).length;
    selection.append(element('span', 'detail-line', `${count} materialization${count === 1 ? '' : 's'} in this workspace`));
    if (data.properties.length) {
      detailLabel(selection, 'PROPERTIES');
      for (const property of data.properties) {
        const kind = property.value_kind;
        const typed = kind && `${kind[0].toUpperCase()}${kind.slice(1)}`;
        const value = kind ? `${property.name} → ${property.many ? `[${typed}]` : typed}` : property.name;
        selection.append(element('span', 'detail-line', value));
      }
    }
    detailLabel(selection, 'DEFINED IN');
    for (const source of data.definitions.slice(0, 3)) selection.append(element('span', 'detail-line', fileName(source.uri)));
  } else {
    selection.append(element('div', 'path', data.id));
    if (data.boundaries.length) {
      detailLabel(selection, 'BOUNDARIES');
      const chips = element('div', 'chips');
      for (const part of data.boundaries) chips.append(element('span', 'chip', `${part.kind} ${part.name}`));
      selection.append(chips);
    }
    if (data.parameters?.length) {
      detailLabel(selection, 'INPUTS');
      for (const parameter of data.parameters) {
        const kind = `${parameter.kind[0].toUpperCase()}${parameter.kind.slice(1)}`;
        selection.append(element('span', 'detail-line', `${parameter.name} → ${parameter.many ? `[${kind}]` : kind}`));
      }
    }
    if (data.fields?.length) {
      detailLabel(selection, 'FIELDS');
      for (const field of data.fields) {
        const files = [...new Set(field.occurrences.map(item => fileName(item.fragment.uri)))];
        const extra = files.length > 2 ? ` + ${files.length - 2} more` : '';
        const count = field.occurrences.length > 1 ? ` · ${field.occurrences.length} assignments` : '';
        selection.append(element('span', 'detail-line', `${field.name}${count} / ${files.slice(0, 2).join(' + ')}${extra}`));
      }
    }
    const bindings = (map.bindings || []).filter(binding => binding.from === data.id);
    if (bindings.length) {
      detailLabel(selection, 'BINDINGS');
      for (const binding of bindings) {
        const target = nodes.find(node => node.id === `materialization:${binding.target}`);
        selection.append(element('span', 'detail-line', `${binding.property} → ${target?.name || binding.target}`));
        for (const argument of binding.arguments) {
          selection.append(element('span', 'detail-line', `${argument.name}: ${argument.values.map(value => value.name).join(', ')}`));
        }
      }
    }
    const related = edges.filter(edge => edge.from === selected || edge.to === selected);
    const relations = related.filter(edge => edge.relation !== 'materializes');
    if (relations.length) {
      detailLabel(selection, 'RELATIONS');
      for (const edge of relations.slice(0, 7)) {
        const other = edge.from === selected ? edge.to : edge.from;
        const arrow = edge.from === selected ? '→' : '←';
        const crossing = edge.crossesBoundary ? ' / crosses boundary' : '';
        selection.append(element('span', 'detail-line', `${edge.relation} ${arrow} ${other.name}${crossing}`));
      }
      if (relations.length > 7) selection.append(element('span', 'detail-line', `+ ${relations.length - 7} more`));
    }
    detailLabel(selection, 'OCCURRENCES');
    if (data.implicit) selection.append(element('span', 'detail-line', 'Inferred from a boundary qualifier'));
    for (const occurrence of data.occurrences.slice(0, 4)) {
      const boundary = occurrence.boundaries.map(part => part.name).join(' · ');
      const access = occurrence.boundaries.length ? occurrence.visibility || 'private' : 'workspace';
      selection.append(element('span', 'detail-line', `${access} / ${fileName(occurrence.source.uri)}${boundary ? ` / ${boundary}` : ''}`));
    }
    if (data.occurrences.length > 4) selection.append(element('span', 'detail-line', `+ ${data.occurrences.length - 4} more`));
  }
  const center = element('button', '', 'CENTER IN VIEW ↗');
  center.type = 'button';
  center.addEventListener('click', () => {
    camera.target = [...selected.position];
    camera.distance = 16;
  });
  selection.append(center);
}

function renderFindings(findings) {
  const container = document.querySelector('#findings');
  container.replaceChildren();
  if (!findings.length) {
    container.append(element('p', 'clear', 'No semantic findings in this workspace.'));
    return;
  }
  const grouped = new Map();
  for (const finding of findings) {
    const key = `${finding.code}\u0000${finding.message}`;
    const group = grouped.get(key);
    if (group) group.count++;
    else grouped.set(key, { finding, count: 1 });
  }
  for (const { finding, count } of [...grouped.values()].slice(0, 6)) {
    const card = element('div', `finding ${finding.severity}`);
    card.append(
      element('strong', '', `${finding.severity.toUpperCase()} · ${finding.code}`),
      element('span', '', finding.message),
      element('small', '', `${fileName(finding.source.uri)}${count > 1 ? ` · ${count} occurrences` : ''}`),
    );
    container.append(card);
  }
  if (grouped.size > 6) container.append(element('p', 'clear', `${grouped.size - 6} more distinct findings in the JSON map.`));
}

function refreshSearch() {
  const query = search.value.trim().toLowerCase();
  results.replaceChildren();
  if (!query) { results.hidden = true; updateGeometry(); return; }
  const matches = nodes.filter(node => node.name.toLowerCase().includes(query) ||
    (node.data.kind || '').toLowerCase().includes(query)).slice(0, 10);
  for (const node of matches) {
    const button = element('button');
    button.type = 'button';
    button.append(element('span', '', node.name), element('small', '', node.category));
    button.addEventListener('click', () => {
      select(node);
      search.value = node.name;
      results.hidden = true;
      search.blur();
    });
    results.append(button);
  }
  if (!matches.length) results.append(element('p', '', 'No matching nodes.'));
  results.hidden = false;
  updateGeometry();
}

function select(node) {
  selected = node;
  renderSelection();
  updateGeometry();
  if (window.matchMedia('(max-width: 760px)').matches && node) {
    setPanelOpen(true);
  }
}

function setPanelOpen(open) {
  inspector.classList.toggle('open', open);
  document.body.classList.toggle('panel-open', open);
  document.querySelector('#mobile-panel-toggle').setAttribute('aria-expanded', String(open));
}

function showTooltip(node, x, y) {
  if (!node) { tooltip.hidden = true; return; }
  tooltip.replaceChildren(element('small', '', node.category), document.createTextNode(node.name));
  tooltip.hidden = false;
  tooltip.style.left = `${Math.min(x + 18, size.width - 170)}px`;
  tooltip.style.top = `${Math.min(y + 18, size.height - 60)}px`;
}

async function loadMap() {
  try {
    const response = await fetch('/api/map', { cache: 'no-store' });
    if (!response.ok) throw new Error(`Map request failed (${response.status})`);
    buildGraph(await response.json());
    loading.hidden = true;
    errorBox.hidden = true;
  } catch (error) {
    loading.hidden = true;
    errorBox.textContent = error.message;
    errorBox.hidden = false;
  }
}

canvas.addEventListener('pointerdown', event => {
  pointer = { id: event.pointerId, x: event.clientX, y: event.clientY, moved: false };
  canvas.setPointerCapture(event.pointerId);
});
canvas.addEventListener('pointermove', event => {
  if (pointer && pointer.id === event.pointerId) {
    const dx = event.clientX - pointer.x;
    const dy = event.clientY - pointer.y;
    if (Math.abs(dx) + Math.abs(dy) > 2) pointer.moved = true;
    camera.yaw += dx * .006;
    camera.pitch = Math.max(-1.25, Math.min(1.25, camera.pitch + dy * .006));
    pointer.x = event.clientX;
    pointer.y = event.clientY;
    tooltip.hidden = true;
    return;
  }
  const rect = canvas.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  hovered = pick(x, y);
  canvas.style.cursor = hovered ? 'pointer' : 'grab';
  showTooltip(hovered, x, y);
});
canvas.addEventListener('pointerup', event => {
  if (!pointer || pointer.id !== event.pointerId) return;
  if (!pointer.moved) {
    const rect = canvas.getBoundingClientRect();
    select(pick(event.clientX - rect.left, event.clientY - rect.top));
  }
  pointer = null;
});
canvas.addEventListener('pointercancel', () => { pointer = null; });
canvas.addEventListener('pointerleave', () => { tooltip.hidden = true; });
canvas.addEventListener('wheel', event => {
  event.preventDefault();
  camera.distance = Math.max(11, Math.min(48, camera.distance * Math.exp(event.deltaY * .001)));
}, { passive: false });
document.querySelector('#reset-view').addEventListener('click', () => {
  camera.yaw = .49;
  camera.pitch = .2;
  camera.distance = 26;
  camera.target = [0, 0, 0];
});
document.querySelector('#toggle-motion').addEventListener('click', event => {
  spinning = !spinning;
  event.currentTarget.textContent = spinning ? 'Ⅱ' : '▶';
  event.currentTarget.title = spinning ? 'Pause rotation' : 'Resume rotation';
  event.currentTarget.setAttribute('aria-label', event.currentTarget.title);
});
document.querySelector('#refresh').addEventListener('click', loadMap);
document.querySelector('#mobile-panel-toggle').addEventListener('click', () => setPanelOpen(true));
document.querySelector('#mobile-panel-close').addEventListener('click', () => setPanelOpen(false));
for (const input of document.querySelectorAll('[data-filter]')) {
  input.addEventListener('change', () => {
    filters[input.dataset.filter] = input.checked;
    updateGeometry();
  });
}
search.addEventListener('input', refreshSearch);
search.addEventListener('keydown', event => {
  if (event.key === 'Escape') { search.value = ''; refreshSearch(); search.blur(); }
  if (event.key === 'Enter' && !results.hidden) results.querySelector('button')?.click();
});
document.addEventListener('keydown', event => {
  if (event.key === '/' && document.activeElement !== search) {
    event.preventDefault();
    search.focus();
  }
  if (event.key === 'Escape' && document.activeElement !== search) {
    select(null);
    setPanelOpen(false);
  }
});

try {
  initGl();
  new ResizeObserver(resize).observe(stage);
  resize();
  requestAnimationFrame(draw);
  loadMap();
} catch (error) {
  loading.hidden = true;
  errorBox.textContent = error.message;
  errorBox.hidden = false;
}

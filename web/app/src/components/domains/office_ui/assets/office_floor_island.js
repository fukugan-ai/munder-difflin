/* Browser-only office island. DTO input only: no Electron bridge, filesystem, or secrets. */

const instances = new WeakMap();
const mountedHosts = new Set();
const mounting = new WeakSet();
const CELL = 32;

function token(host, name, fallback) {
  const value = getComputedStyle(host).getPropertyValue(name).trim();
  return value || fallback;
}

function readDto(host) {
  const agents = [...host.querySelectorAll("[data-office-agent]")].map((node) => ({
    id: node.dataset.id || "",
    name: node.dataset.name || "",
    character: node.dataset.character || "jim",
    accent: node.dataset.accent || "sky",
    status: node.dataset.status || "idle",
    action: node.dataset.action || node.dataset.lastPrompt || "",
    carrying: node.dataset.carrying || "",
    isGod: node.dataset.isGod === "true",
  }));
  const tasks = [...host.querySelectorAll("[data-office-task]")].map((node) => ({
    id: node.dataset.id || "",
    status: node.dataset.status || "todo",
    assignee: node.dataset.assignee || "",
    humanQuestion: node.dataset.humanQuestion === "true",
  }));
  return {
    revision: Number(host.dataset.revision || 0),
    theme: host.dataset.themeId || "office",
    selected: host.dataset.selectedAgent || "",
    paused: host.dataset.paused === "true",
    agents,
    tasks,
  };
}

function emit(host, type, data = undefined) {
  host.dispatchEvent(new CustomEvent("office-ui-action", {
    bubbles: true,
    composed: true,
    detail: data === undefined ? { type } : { type, data },
  }));
}

function errorMessage(error) {
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

function loadClassicScript(url, ready) {
  if (ready()) return Promise.resolve();
  if (!url) return Promise.reject(new Error("office dependency URL is missing"));
  const existing = [...document.scripts].find((script) => script.dataset.officeDependency === url);
  if (existing) {
    return new Promise((resolve, reject) => {
      existing.addEventListener("load", () => ready() ? resolve() : reject(new Error(`dependency did not initialize: ${url}`)), { once: true });
      existing.addEventListener("error", () => reject(new Error(`dependency load failed: ${url}`)), { once: true });
    });
  }
  return new Promise((resolve, reject) => {
    const script = document.createElement("script");
    script.src = url;
    script.async = false;
    script.dataset.officeDependency = url;
    script.addEventListener("load", () => ready() ? resolve() : reject(new Error(`dependency did not initialize: ${url}`)), { once: true });
    script.addEventListener("error", () => reject(new Error(`dependency load failed: ${url}`)), { once: true });
    document.head.appendChild(script);
  });
}

async function loadDependencies(host) {
  host.dataset.pixiState = "loading";
  await loadClassicScript(
    host.dataset.portraitArtSrc,
    () => Boolean(globalThis.OfficePortraitArt?.paintPortrait),
  );
  await loadClassicScript(
    host.dataset.darrylArtSrc,
    () => globalThis.OfficePortraitArt?.darrylReady === true,
  );
  await loadClassicScript(
    host.dataset.pixiSrc,
    () => Boolean(globalThis.PIXI?.Application && globalThis.PIXI?.Graphics && globalThis.PIXI?.Text),
  );
  host.dataset.artState = "ready";
  host.dataset.pixiState = "ready";
}

class CanvasFloor {
  constructor(host, canvas) {
    this.host = host;
    this.canvas = canvas;
    this.context = canvas.getContext("2d", { alpha: false });
    this.dto = readDto(host);
    this.hitboxes = [];
    this.envelopes = [];
    this.frame = 0;
    this.resizeObserver = new ResizeObserver(() => this.resize());
    this.resizeObserver.observe(host);
    this.onClick = (event) => this.click(event);
    canvas.addEventListener("click", this.onClick);
    this.onHandoff = (event) => this.handoff(event.detail);
    host.addEventListener("office-handoff", this.onHandoff);
    this.resize();
    this.loop = this.loop.bind(this);
    this.frame = requestAnimationFrame(this.loop);
  }

  update(dto) {
    this.dto = dto;
    if (dto.paused) this.draw();
  }

  resize() {
    const rect = this.host.getBoundingClientRect();
    const density = Math.min(devicePixelRatio || 1, 2);
    this.canvas.width = Math.max(1, Math.round(rect.width * density));
    this.canvas.height = Math.max(1, Math.round(rect.height * density));
    this.canvas.style.width = `${rect.width}px`;
    this.canvas.style.height = `${rect.height}px`;
    this.context?.setTransform(density, 0, 0, density, 0, 0);
    this.draw();
  }

  loop() {
    if (!this.dto.paused) {
      this.updateEnvelopes();
      this.draw();
    }
    this.frame = requestAnimationFrame(this.loop);
  }

  palette() {
    return {
      ground: token(this.host, "--color-ink-900", "#1a1320"),
      paper: token(this.host, "--color-paper-100", "#fcfaf0"),
      ink: token(this.host, "--color-ink-900", "#1a1320"),
      quiet: token(this.host, "--color-ink-500", "#6b5878"),
      cream: token(this.host, "--color-cream-200", "#f4e9c7"),
      border: token(this.host, "--color-ink-300", "#a899b5"),
      coral: token(this.host, "--color-coral", "#d96a62"),
      mint: token(this.host, "--color-mint", "#5ca97a"),
      sky: token(this.host, "--color-sky", "#4f9faf"),
      lemon: token(this.host, "--color-lemon", "#dcab3c"),
      lilac: token(this.host, "--color-lilac", "#9482d3"),
    };
  }

  draw() {
    const context = this.context;
    if (!context) return;
    const width = this.host.clientWidth;
    const height = this.host.clientHeight;
    const colors = this.palette();
    context.clearRect(0, 0, width, height);
    context.fillStyle = colors.ground;
    context.fillRect(0, 0, width, height);

    const margin = Math.max(16, Math.floor(Math.min(width, height) * 0.04));
    const room = { x: margin, y: margin, width: width - margin * 2, height: height - margin * 2 };
    context.fillStyle = colors.cream;
    context.fillRect(room.x, room.y, room.width, room.height);
    context.strokeStyle = colors.border;
    context.lineWidth = 1;
    for (let x = room.x; x <= room.x + room.width; x += CELL) {
      context.beginPath(); context.moveTo(x, room.y); context.lineTo(x, room.y + room.height); context.stroke();
    }
    for (let y = room.y; y <= room.y + room.height; y += CELL) {
      context.beginPath(); context.moveTo(room.x, y); context.lineTo(room.x + room.width, y); context.stroke();
    }

    this.hitboxes = [];
    this.drawBoards(context, colors, room);
    this.drawDesks(context, colors, room);
    this.drawAgents(context, colors, room);
    this.drawEnvelopes(context, colors);
  }

  drawBoards(context, colors, room) {
    const tasks = this.dto.tasks;
    const todo = tasks.filter((task) => task.status === "todo").length;
    const blocked = tasks.filter((task) => task.status === "blocked").length;
    const questions = tasks.filter((task) => task.humanQuestion).length;
    const boards = [
      { x: room.x + 24, label: `BLOCK ${blocked}`, color: colors.coral, action: "open_tasks" },
      { x: room.x + 112, label: `TODO ${todo}`, color: colors.lemon, action: "open_tasks" },
      { x: room.x + 200, label: `ASK ${questions}`, color: colors.lilac, action: "open_human_questions" },
    ];
    for (const board of boards) {
      const y = room.y + 12;
      context.fillStyle = board.color;
      context.fillRect(board.x, y, 76, 28);
      context.fillStyle = colors.ink;
      context.font = "10px ui-monospace, monospace";
      context.fillText(board.label, board.x + 6, y + 18);
      this.hitboxes.push({ x: board.x, y, width: 76, height: 28, action: board.action });
    }
  }

  drawDesks(context, colors, room) {
    const columns = Math.max(2, Math.min(5, Math.floor(room.width / 180)));
    const rows = Math.max(1, Math.ceil(Math.max(1, this.dto.agents.length) / columns));
    for (let index = 0; index < Math.max(4, rows * columns); index += 1) {
      const column = index % columns;
      const row = Math.floor(index / columns);
      const x = room.x + 48 + column * ((room.width - 96) / columns);
      const y = room.y + 112 + row * Math.min(120, (room.height - 150) / rows);
      context.fillStyle = colors.quiet;
      context.fillRect(x, y, 84, 30);
      context.fillStyle = colors.paper;
      context.fillRect(x + 30, y - 12, 24, 14);
    }
  }

  drawAgents(context, colors, room) {
    const columns = Math.max(2, Math.min(5, Math.floor(room.width / 180)));
    this.dto.agents.forEach((agent, index) => {
      const column = index % columns;
      const row = Math.floor(index / columns);
      const baseX = room.x + 72 + column * ((room.width - 96) / columns);
      const baseY = room.y + 96 + row * Math.min(120, (room.height - 150) / Math.max(1, Math.ceil(this.dto.agents.length / columns)));
      const bob = agent.status === "working" || agent.status === "thinking"
        ? Math.round(Math.sin(performance.now() / 260 + index) * 2)
        : 0;
      const x = baseX;
      const y = baseY + bob;
      const accent = colors[agent.accent] || colors.sky;
      context.fillStyle = accent;
      context.fillRect(x, y, 28, 34);
      context.fillStyle = colors.ink;
      context.font = "bold 12px ui-monospace, monospace";
      context.fillText((agent.name || "?").slice(0, 1).toUpperCase(), x + 9, y + 21);
      context.fillStyle = colors.paper;
      context.fillRect(x - 12, y - 22, Math.max(54, agent.name.length * 7), 17);
      context.fillStyle = colors.ink;
      context.font = "10px system-ui, sans-serif";
      context.fillText(agent.name, x - 7, y - 10);
      if (agent.action) {
        context.fillStyle = colors.paper;
        context.fillRect(x + 20, y + 2, Math.min(160, agent.action.length * 6 + 12), 18);
        context.fillStyle = colors.ink;
        context.fillText(agent.action.slice(0, 24), x + 26, y + 15);
      }
      if (agent.id === this.dto.selected) {
        context.strokeStyle = colors.paper;
        context.lineWidth = 2;
        context.strokeRect(x - 3, y - 3, 34, 40);
      }
      this.hitboxes.push({ x: x - 8, y: y - 24, width: 52, height: 64, action: "select_agent", id: agent.id });
      agent._position = { x: x + 14, y: y + 17 };
    });
  }

  handoff(detail) {
    if (!detail || typeof detail.from !== "string" || !Array.isArray(detail.targets)) return;
    for (const target of detail.targets) {
      this.envelopes.push({ from: detail.from, to: String(target), progress: 0 });
    }
  }

  updateEnvelopes() {
    for (const envelope of this.envelopes) envelope.progress += 0.015;
    this.envelopes = this.envelopes.filter((envelope) => envelope.progress < 1);
  }

  drawEnvelopes(context, colors) {
    const positions = new Map(this.dto.agents.map((agent) => [agent.id, agent._position]));
    for (const envelope of this.envelopes) {
      const from = positions.get(envelope.from);
      const to = positions.get(envelope.to);
      if (!from || !to) continue;
      const t = envelope.progress;
      const x = from.x + (to.x - from.x) * t;
      const y = from.y + (to.y - from.y) * t - Math.sin(Math.PI * t) * 32;
      context.fillStyle = colors.paper;
      context.fillRect(x - 6, y - 4, 12, 8);
      context.strokeStyle = colors.ink;
      context.strokeRect(x - 6, y - 4, 12, 8);
    }
  }

  click(event) {
    const rect = this.canvas.getBoundingClientRect();
    const x = event.clientX - rect.left;
    const y = event.clientY - rect.top;
    const hit = [...this.hitboxes].reverse().find((box) =>
      x >= box.x && x <= box.x + box.width && y >= box.y && y <= box.y + box.height
    );
    if (!hit) return;
    if (hit.action === "select_agent") emit(this.host, "select_agent", { agent_id: hit.id });
    else emit(this.host, hit.action);
  }

  destroy() {
    cancelAnimationFrame(this.frame);
    this.resizeObserver.disconnect();
    this.canvas.removeEventListener("click", this.onClick);
    this.host.removeEventListener("office-handoff", this.onHandoff);
  }
}

const TILE_ID_MASK = 0x1fffffff;
const FLIPPED_H_FLAG = 0x80000000;
const FLIPPED_V_FLAG = 0x40000000;
const FLIPPED_D_FLAG = 0x20000000;
const MAP_LAYERS = ["floor", "walls", "furniture-below", "furniture-above"];
const OFFICE_SEATS = [
  "desk-ceo", "pc-1", "pc-2", "pc-3", "pc-4", "pc-5", "pc-6",
  "desk-chief-architect", "desk-product-manager", "desk-team-lead",
  "desk-backend-engineer", "desk-ui-ux-expert", "desk-data-engineer",
  "desk-project-manager", "desk-market-researcher", "desk-agent-organizer",
];

function buildMapModel(map) {
  const collision = map.layers.find((layer) => layer.name === "collision" && layer.type === "tilelayer");
  const grid = Array.from({ length: map.height }, (_, y) =>
    Array.from({ length: map.width }, (_, x) =>
      !collision?.data?.[y * map.width + x] ||
      (collision.data[y * map.width + x] & TILE_ID_MASK) === 0
    )
  );
  const objects = map.layers.find((layer) => layer.name === "spawn-points" && layer.type === "objectgroup");
  const spawns = new Map((objects?.objects || []).map((object) => [object.name, {
    x: Math.floor(object.x / map.tilewidth),
    y: Math.floor(object.y / map.tileheight),
  }]));
  for (const [name, point] of spawns) {
    if (["desk-", "pc-", "warroom-", "entrance"].some((prefix) => name.startsWith(prefix))) {
      if (grid[point.y]?.[point.x] !== undefined) grid[point.y][point.x] = true;
    }
  }
  return {
    map,
    spawns,
    isWalkable(x, y) {
      return Boolean(x >= 0 && y >= 0 && x < map.width && y < map.height && grid[y][x]);
    },
  };
}

function findPath(model, start, goal) {
  if (start.x === goal.x && start.y === goal.y) return [];
  if (!model.isWalkable(goal.x, goal.y)) return null;
  const key = (point) => `${point.x},${point.y}`;
  const queue = [start];
  let cursor = 0;
  const visited = new Set([key(start)]);
  const parents = new Map();
  const directions = [[0, -1], [0, 1], [-1, 0], [1, 0]];
  while (cursor < queue.length) {
    const current = queue[cursor++];
    for (const [dx, dy] of directions) {
      const next = { x: current.x + dx, y: current.y + dy };
      const nextKey = key(next);
      if (visited.has(nextKey) || !model.isWalkable(next.x, next.y)) continue;
      visited.add(nextKey);
      parents.set(nextKey, current);
      if (next.x === goal.x && next.y === goal.y) {
        const path = [];
        let step = next;
        while (step.x !== start.x || step.y !== start.y) {
          path.unshift(step);
          step = parents.get(key(step));
        }
        return path;
      }
      queue.push(next);
    }
  }
  return null;
}

function patchTilesets(map) {
  return [
    map.tilesets[0],
    { firstgid: 513, imagewidth: 256, imageheight: 512, tilewidth: 16, tileheight: 16, columns: 16, tilecount: 512 },
    { firstgid: 1025, imagewidth: 256, imageheight: 1424, tilewidth: 16, tileheight: 16, columns: 16, tilecount: 1424 },
  ];
}

function tileTexture(PIXI, rawGid, tilesets, textures) {
  const gid = rawGid & TILE_ID_MASK;
  if (!gid) return null;
  let index = -1;
  for (let candidate = tilesets.length - 1; candidate >= 0; candidate -= 1) {
    if (gid >= tilesets[candidate].firstgid) {
      index = candidate;
      break;
    }
  }
  if (index < 0 || !textures[index]) return null;
  const tileset = tilesets[index];
  const tileWidth = tileset.tilewidth || 16;
  const tileHeight = tileset.tileheight || 16;
  const columns = tileset.columns || Math.floor((tileset.imagewidth || 256) / tileWidth);
  const local = gid - tileset.firstgid;
  const frame = new PIXI.Rectangle(
    (local % columns) * tileWidth,
    Math.floor(local / columns) * tileHeight,
    tileWidth,
    tileHeight,
  );
  return new PIXI.Texture({ source: textures[index].source, frame });
}

function addTiledLayers(PIXI, root, map, textures) {
  const tilesets = patchTilesets(map);
  for (const layerName of MAP_LAYERS) {
    const layer = map.layers.find((candidate) => candidate.name === layerName && candidate.type === "tilelayer");
    const container = new PIXI.Container();
    container.label = layerName;
    for (let y = 0; y < map.height; y += 1) {
      for (let x = 0; x < map.width; x += 1) {
        const raw = layer?.data?.[y * map.width + x] || 0;
        const texture = tileTexture(PIXI, raw, tilesets, textures);
        if (!texture) continue;
        const sprite = new PIXI.Sprite(texture);
        const flipH = (raw & FLIPPED_H_FLAG) !== 0;
        const flipV = (raw & FLIPPED_V_FLAG) !== 0;
        const flipD = (raw & FLIPPED_D_FLAG) !== 0;
        if (flipH || flipV || flipD) {
          sprite.anchor.set(0.5);
          sprite.position.set(x * map.tilewidth + map.tilewidth / 2, y * map.tileheight + map.tileheight / 2);
          if (flipD) {
            sprite.rotation = Math.PI / 2;
            if (!flipH) sprite.scale.x = -1;
            if (flipV) sprite.scale.y = -1;
          } else {
            if (flipH) sprite.scale.x = -1;
            if (flipV) sprite.scale.y = -1;
          }
        } else {
          sprite.position.set(x * map.tilewidth, y * map.tileheight);
        }
        container.addChild(sprite);
      }
    }
    root.addChild(container);
  }
}

class SeatPool {
  constructor(seats) {
    this.seats = seats;
    this.claimed = new Set();
  }

  reserveNext() {
    const seat = this.seats.find((candidate) => !this.claimed.has(candidate));
    if (!seat) return null;
    this.claimed.add(seat);
    return seat;
  }

  release(seat) {
    this.claimed.delete(seat);
  }
}

const sceneTextureCache = new Map();

function castTextures(PIXI, name) {
  const art = globalThis.OfficePortraitArt;
  if (sceneTextureCache.has(name)) return sceneTextureCache.get(name);
  if (!art?.sceneFrameBufs) return null;
  const source = art.sceneFrameBufs(name);
  const build = (buffer) => {
    const canvas = document.createElement("canvas");
    canvas.width = art.SCENE_W;
    canvas.height = art.SCENE_H;
    const context = canvas.getContext("2d");
    const image = context.createImageData(art.SCENE_W, art.SCENE_H);
    image.data.set(buffer);
    context.putImageData(image, 0, 0);
    const texture = PIXI.Texture.from(canvas);
    texture.source.scaleMode = "nearest";
    return texture;
  };
  const textures = {
    front: source.front.map(build),
    back: source.back.map(build),
  };
  sceneTextureCache.set(name, textures);
  return textures;
}

class MapCharacter {
  constructor(PIXI, host, agent, start, path, tileSize, model, home) {
    this.host = host;
    this.agent = agent;
    this.path = path;
    this.tileSize = tileSize;
    this.model = model;
    this.home = home;
    this.away = false;
    this.returnAt = Number.POSITIVE_INFINITY;
    this.elapsed = 0;
    this.container = new PIXI.Container();
    this.container.eventMode = "static";
    this.container.cursor = "pointer";
    this.container.on("pointertap", () => emit(host, "select_agent", { agent_id: this.agent.id }));
    this.textures = castTextures(PIXI, agent.character);
    this.body = this.textures
      ? new PIXI.Sprite(this.textures.front[0])
      : new PIXI.Graphics().rect(-6, -24, 12, 24).fill(0x4f9faf);
    if (this.body.anchor) this.body.anchor.set(0.5, 1);
    this.body.scale.set(1.08);
    this.selection = new PIXI.Graphics();
    this.bubble = new PIXI.Container();
    this.bubbleBackground = new PIXI.Graphics();
    this.action = new PIXI.Text({
      text: "",
      style: { fill: 0x17121b, fontFamily: "ui-monospace, monospace", fontSize: 7, fontWeight: "600" },
    });
    this.bubble.addChild(this.bubbleBackground, this.action);
    this.container.addChild(this.selection, this.body, this.bubble);
    this.container.position.set((start.x + 0.5) * tileSize, (start.y + 1) * tileSize);
    this.paint(false);
  }

  setAgent(agent, selected) {
    this.agent = agent;
    this.action.text = (agent.action || statusBubble(agent.status)).slice(0, 44);
    const width = Math.max(22, this.action.width + 8);
    this.bubbleBackground.clear().roundRect(-4, -3, width, 13, 2).fill(0xfffcf2).stroke({ color: 0x26222e, width: 1 });
    this.bubbleBackground.circle(1, 13, 1.5).fill(0xfffcf2).stroke({ color: 0x26222e, width: 0.7 });
    this.bubbleBackground.circle(-2, 17, 1).fill(0xfffcf2).stroke({ color: 0x26222e, width: 0.7 });
    this.bubble.position.set(-width / 2, -43);
    if (agent.status !== "idle" && this.away) {
      this.setDestination(this.home);
      this.away = false;
    }
    this.paint(selected);
  }

  setDestination(target) {
    const current = {
      x: Math.floor(this.container.x / this.tileSize),
      y: Math.floor(this.container.y / this.tileSize),
    };
    this.path = findPath(this.model, current, target) || [];
  }

  startErrand(target) {
    if (this.agent.status !== "idle" || this.path.length) return;
    this.setDestination(target);
    if (this.path.length) {
      this.away = true;
      this.returnAt = Number.POSITIVE_INFINITY;
    }
  }

  paint(selected, walking = false, back = false) {
    this.selection.clear();
    if (selected) this.selection.ellipse(0, -2, 12, 5).stroke({ color: 0xf2df8a, width: 2 });
    if (this.textures) {
      const frames = back ? this.textures.back : this.textures.front;
      this.body.texture = frames[walking ? Math.floor(this.elapsed / 130) % 3 : 0];
    }
  }

  tick(deltaMs, selected) {
    this.elapsed += deltaMs;
    if (this.path.length) {
      const target = this.path[0];
      const tx = (target.x + 0.5) * this.tileSize;
      const ty = (target.y + 1) * this.tileSize;
      const dx = tx - this.container.x;
      const dy = ty - this.container.y;
      const distance = Math.hypot(dx, dy);
      const step = Math.min(distance, deltaMs * 0.035);
      if (distance <= 0.6) {
        this.path.shift();
        if (!this.path.length && this.away) this.returnAt = this.elapsed + 3600;
      }
      else this.container.position.set(this.container.x + dx / distance * step, this.container.y + dy / distance * step);
      this.back = dy < -0.1;
      this.paint(selected, true, this.back);
    } else {
      if (this.away && this.elapsed >= this.returnAt) {
        this.setDestination(this.home);
        this.away = false;
      }
      const active = ["thinking", "working", "compacting", "looping"].includes(this.agent.status);
      this.container.y += active ? Math.sin(this.elapsed / 180) * 0.03 : 0;
      this.paint(selected, active, this.back);
    }
    this.container.zIndex = this.container.y;
  }
}

function statusBubble(status) {
  return {
    idle: "idle",
    thinking: "thinking",
    working: "working",
    waiting: "awaiting",
    blocked: "needs help",
    success: "done",
    ghost: "away",
    compacting: "compacting",
    looping: "looping",
  }[status] || status;
}

class PixiFloor {
  static async create(host, fallbackCanvas) {
    const PIXI = globalThis.PIXI;
    const app = new PIXI.Application();
    await app.init({
      resizeTo: host,
      antialias: false,
      autoDensity: true,
      resolution: Math.min(devicePixelRatio || 1, 2),
      background: token(host, "--color-ink-900", "#1a1320"),
    });
    const floor = new PixiFloor(host, app);
    try {
      await floor.load();
      app.canvas.classList.add("pixi-office-canvas");
      host.insertBefore(app.canvas, fallbackCanvas);
      fallbackCanvas.hidden = true;
      floor.fit();
      return floor;
    } catch (error) {
      floor.destroy();
      fallbackCanvas.hidden = false;
      throw error;
    }
  }

  constructor(host, app) {
    this.host = host;
    this.app = app;
    this.dto = readDto(host);
    this.characters = new Map();
    this.assignments = new Map();
    this.envelopes = [];
    this.onHandoff = (event) => this.handoff(event.detail);
    host.addEventListener("office-handoff", this.onHandoff);
    this.resizeObserver = new ResizeObserver(() => this.fit());
    this.resizeObserver.observe(host);
    this.app.ticker.add((ticker) => this.tick(ticker.deltaMS));
    this.errandElapsed = 0;
  }

  async load() {
    const PIXI = globalThis.PIXI;
    const loadRevision = (this.loadRevision || 0) + 1;
    this.loadRevision = loadRevision;
    const mapUrl = this.dto.theme === "brooklyn99" ? this.host.dataset.brooklyn99Map : this.host.dataset.officeMap;
    const textureUrls = [
      this.host.dataset.officeTileset,
      this.host.dataset.officeFloorsWalls,
      this.host.dataset.officeInteriors,
    ];
    const [mapResponse, loadedTextures] = await Promise.all([
      fetch(mapUrl, { credentials: "same-origin" }),
      Promise.all(textureUrls.map((url) => PIXI.Assets.load(url))),
    ]);
    if (!mapResponse.ok) throw new Error(`office map fetch failed: ${mapResponse.status}`);
    const nextMap = await mapResponse.json();
    if (loadRevision !== this.loadRevision) return;
    if (this.root) {
      this.app.stage.removeChild(this.root);
      this.root.destroy({ children: true });
    }
    this.characters.clear();
    this.assignments.clear();
    this.envelopes = [];
    this.map = nextMap;
    this.model = buildMapModel(this.map);
    this.root = new PIXI.Container();
    this.root.sortableChildren = true;
    addTiledLayers(PIXI, this.root, this.map, loadedTextures);
    this.characterLayer = new PIXI.Container();
    this.characterLayer.sortableChildren = true;
    this.root.addChild(this.characterLayer);
    this.app.stage.addChild(this.root);
    this.seats = new SeatPool(OFFICE_SEATS.filter((seat) => this.model.spawns.has(seat)));
    this.makeAnchors();
    this.syncCharacters();
    this.fit();
    this.host.dataset.mapLoaded = this.dto.theme === "brooklyn99" ? "brooklyn99" : "office";
  }

  makeAnchors() {
    const PIXI = globalThis.PIXI;
    const anchors = [
      [6, 10, "open_tasks"],
      [4, 1, "open_tasks"],
      [1, 1, "request_close"],
    ];
    for (const [x, y, action] of anchors) {
      const target = new PIXI.Graphics().rect(x * this.map.tilewidth, y * this.map.tileheight, 32, 32).fill({ color: 0xffffff, alpha: 0.001 });
      target.eventMode = "static";
      target.cursor = "pointer";
      target.on("pointertap", () => emit(this.host, action));
      this.root.addChild(target);
    }
  }

  update(dto) {
    const themeChanged = dto.theme !== this.dto.theme;
    this.dto = dto;
    if (dto.paused) this.app.ticker.stop();
    else this.app.ticker.start();
    if (themeChanged) {
      this.load().catch((error) => {
        this.host.dataset.mapError = "theme";
        console.warn("[office-island] theme load failed; keeping current map", error);
      });
    }
    if (this.model) this.syncCharacters();
  }

  syncCharacters() {
    const live = new Set(this.dto.agents.map((agent) => agent.id));
    for (const [id, character] of this.characters) {
      if (live.has(id)) continue;
      character.container.destroy({ children: true });
      this.characters.delete(id);
      this.seats.release(this.assignments.get(id));
      this.assignments.delete(id);
    }
    const entrance = this.model.spawns.get("entrance") || { x: 18, y: 20 };
    for (const agent of this.dto.agents) {
      let character = this.characters.get(agent.id);
      if (!character) {
        const seatName = this.seats.reserveNext();
        const seat = this.model.spawns.get(seatName) || entrance;
        const path = findPath(this.model, entrance, seat) || [];
        character = new MapCharacter(
          globalThis.PIXI,
          this.host,
          agent,
          entrance,
          path,
          this.map.tilewidth,
          this.model,
          seat,
        );
        this.assignments.set(agent.id, seatName);
        this.characters.set(agent.id, character);
        this.characterLayer.addChild(character.container);
      }
      character.setAgent(agent, agent.id === this.dto.selected);
    }
  }

  fit() {
    if (!this.root || !this.map) return;
    const mapWidth = this.map.width * this.map.tilewidth;
    const mapHeight = this.map.height * this.map.tileheight;
    const scale = Math.max(0.5, Math.min(this.host.clientWidth / mapWidth, this.host.clientHeight / mapHeight));
    this.root.scale.set(scale);
    this.root.position.set(
      Math.round((this.host.clientWidth - mapWidth * scale) / 2),
      Math.round((this.host.clientHeight - mapHeight * scale) / 2),
    );
  }

  tick(deltaMs) {
    for (const character of this.characters.values()) {
      character.tick(deltaMs, character.agent.id === this.dto.selected);
    }
    this.errandElapsed += deltaMs;
    if (this.errandElapsed >= 4200) {
      this.errandElapsed = 0;
      const idle = [...this.characters.values()].filter((character) =>
        character.agent.status === "idle" && !character.path.length && !character.away
      );
      const errands = [
        { x: 2, y: 20 }, { x: 22, y: 20 }, { x: 30, y: 20 },
        { x: 10, y: 3 }, { x: 16, y: 3 }, { x: 29, y: 20 },
        { x: 18, y: 20 }, { x: 31, y: 16 },
      ].filter((point) => this.model.isWalkable(point.x, point.y));
      if (idle.length && errands.length) {
        const character = idle[Math.floor(Math.random() * idle.length)];
        character.startErrand(errands[Math.floor(Math.random() * errands.length)]);
      }
    }
    for (const envelope of this.envelopes) {
      envelope.progress += deltaMs / 1300;
      const from = this.characters.get(envelope.from)?.container;
      const to = this.characters.get(envelope.to)?.container;
      if (!from || !to) continue;
      const t = Math.min(1, envelope.progress);
      envelope.sprite.position.set(
        from.x + (to.x - from.x) * t,
        from.y + (to.y - from.y) * t - Math.sin(Math.PI * t) * 28,
      );
    }
    const finished = this.envelopes.filter((envelope) => envelope.progress >= 1);
    this.envelopes = this.envelopes.filter((envelope) => envelope.progress < 1);
    for (const envelope of finished) envelope.sprite.destroy();
  }

  handoff(detail) {
    if (!detail || typeof detail.from !== "string" || !Array.isArray(detail.targets)) return;
    const PIXI = globalThis.PIXI;
    for (const target of detail.targets) {
      const sprite = new PIXI.Graphics().rect(-5, -3, 10, 7).fill(0xfff7df).stroke({ color: 0x17121b, width: 1 });
      this.characterLayer.addChild(sprite);
      this.envelopes.push({ from: detail.from, to: String(target), progress: 0, sprite });
    }
  }

  destroy() {
    this.host.removeEventListener("office-handoff", this.onHandoff);
    this.resizeObserver.disconnect();
    this.app.destroy(true, { children: true, texture: false, textureSource: false });
  }
}

async function mount(host) {
  if (instances.has(host) || mounting.has(host)) return;
  mounting.add(host);
  const canvas = host.querySelector(".office-island__fallback");
  if (!(canvas instanceof HTMLCanvasElement)) {
    mounting.delete(host);
    return;
  }
  let floor;
  try {
    await loadDependencies(host);
    if (globalThis.PIXI?.Application && globalThis.PIXI?.Graphics && globalThis.PIXI?.Text) {
      floor = await PixiFloor.create(host, canvas);
    } else {
      throw new Error("Pixi runtime is unavailable after dependency load");
    }
  } catch (error) {
    const message = errorMessage(error);
    host.dataset.pixiState = "error";
    host.dataset.loadError = message;
    const status = host.querySelector(".office-island__status");
    if (status) status.textContent = `Pixiの読込に失敗しました: ${message}`;
    canvas.hidden = false;
    console.error("[office-island] Pixi initialization failed; using Canvas2D", error);
    floor = new CanvasFloor(host, canvas);
  }
  instances.set(host, floor);
  mountedHosts.add(host);
  mounting.delete(host);
  host.dataset.renderer = floor instanceof PixiFloor ? "pixi" : "canvas-fallback";
  host.dataset.runtimeMarker = floor instanceof PixiFloor ? "pixi-tmj-ready" : "canvas-degraded";
  host.dataset.ready = "true";
}

function paintPortraits() {
  const art = globalThis.OfficePortraitArt;
  if (!art?.paintPortrait) return;
  for (const canvas of document.querySelectorAll("canvas[data-office-portrait]")) {
    const character = canvas.dataset.officePortrait || "jim";
    if (canvas.dataset.paintedCharacter === character) continue;
    const context = canvas.getContext("2d");
    if (!context) continue;
    art.paintPortrait(context, character, 2);
    canvas.dataset.paintedCharacter = character;
  }
}

function refresh() {
  paintPortraits();
  const live = new Set(document.querySelectorAll("[data-office-island]"));
  for (const host of mountedHosts) {
    if (!live.has(host)) {
      instances.get(host)?.destroy();
      instances.delete(host);
      mountedHosts.delete(host);
    }
  }
  for (const host of live) {
    mount(host);
    instances.get(host)?.update(readDto(host));
  }
}

const observer = new MutationObserver(refresh);
observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true });
refresh();

export { readDto };

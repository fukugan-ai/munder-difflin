/* Distinct Darryl portrait/walk art layered on the repository's procedural cast composer. */
(() => {
  const art = globalThis.OfficePortraitArt;
  if (!art?.paintPortrait || !art?.sceneFrameBufs || art.darrylReady) return;

  const originalPaintPortrait = art.paintPortrait.bind(art);
  const originalSceneFrameBufs = art.sceneFrameBufs.bind(art);
  const sceneCache = new Map();

  const sourceShirt = [
    [183, 146, 105],
    [150, 120, 86],
    [102, 82, 58],
  ];
  const darrylShirt = [
    [102, 139, 173],
    [78, 112, 145],
    [53, 76, 99],
  ];

  function recolor(buffer) {
    const result = new Uint8ClampedArray(buffer);
    for (let offset = 0; offset < result.length; offset += 4) {
      for (let index = 0; index < sourceShirt.length; index += 1) {
        const source = sourceShirt[index];
        if (
          result[offset] === source[0]
          && result[offset + 1] === source[1]
          && result[offset + 2] === source[2]
        ) {
          const target = darrylShirt[index];
          result[offset] = target[0];
          result[offset + 1] = target[1];
          result[offset + 2] = target[2];
          break;
        }
      }
    }
    return result;
  }

  function darrylFrames() {
    let frames = sceneCache.get("darryl");
    if (frames) return frames;
    const stanley = originalSceneFrameBufs("stanley");
    frames = {
      front: stanley.front.map(recolor),
      back: stanley.back.map(recolor),
    };
    sceneCache.set("darryl", frames);
    return frames;
  }

  art.sceneFrameBufs = (name) => (
    name === "darryl" ? darrylFrames() : originalSceneFrameBufs(name)
  );
  art.paintPortrait = (context, name, scale = 2) => {
    if (name !== "darryl") {
      originalPaintPortrait(context, name, scale);
      return;
    }
    const buffer = darrylFrames().front[0];
    const source = document.createElement("canvas");
    source.width = art.SCENE_W;
    source.height = art.SCENE_H;
    const sourceContext = source.getContext("2d");
    const image = sourceContext.createImageData(art.SCENE_W, art.SCENE_H);
    image.data.set(buffer);
    sourceContext.putImageData(image, 0, 0);
    context.imageSmoothingEnabled = false;
    context.clearRect(0, 0, art.PORTRAIT_W * scale, art.PORTRAIT_H * scale);
    context.drawImage(
      source,
      0,
      0,
      art.PORTRAIT_W,
      art.PORTRAIT_H,
      0,
      0,
      art.PORTRAIT_W * scale,
      art.PORTRAIT_H * scale,
    );
  };
  art.darrylReady = true;
})();

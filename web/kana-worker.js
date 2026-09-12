import init, { KanaEngine, kana_sample_zip } from "./pkg/kana/idiosepius_kana_web.js";

const ready = init().then(() => new KanaEngine());
self.onmessage = async ({ data }) => {
  const { id, action, script, strokes, samples } = data;
  try {
    const engine = await ready;
    let value;
    if (action === "recognize") value = JSON.parse(engine.recognize_pair(JSON.stringify(strokes), script));
    else if (action === "init") value = {
      inventory: engine.inventory(script), metadata: JSON.parse(engine.metadata(script)),
    };
    else if (action === "export") value = kana_sample_zip(JSON.stringify(samples.map((sample) => JSON.stringify(sample))));
    else throw new Error("Unknown kana request");
    self.postMessage({ id, value });
  } catch (error) {
    self.postMessage({ id, error: String(error) });
  }
};

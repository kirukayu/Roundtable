/**
 * Volumetric monochrome fog.
 *
 * Four enormous masses, three light and one dark, blurred past recognition and
 * drifting over sixty to ninety seconds. The dark one matters: without something
 * subtracting, the field creeps uniformly brighter and stops reading as smoke.
 *
 * Nothing here is interactive and nothing carries colour.
 */
export function Fog() {
  return (
    <>
      <div className="fog" aria-hidden="true">
        <span className="fog__m fog__m--1" />
        <span className="fog__m fog__m--2" />
        <span className="fog__m fog__m--3" />
        <span className="fog__m fog__m--4" />
      </div>
      <div className="grain" aria-hidden="true" />
    </>
  );
}

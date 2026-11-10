// reader-loupe.js — hold-to-magnify loupe (Houdoku/Yomikiru loupe ideas, reimplemented).

export class ReaderLoupe {
  /**
   * @param {HTMLElement} host
   * @param {{ glass?: HTMLElement, scale?: number }} opts
   */
  constructor(host, { glass = null, scale = 2.2 } = {}) {
    this.host = host;
    this.scale = scale;
    this.el = glass || document.createElement('div');
    this.el.className = 'reader-loupe';
    this.el.hidden = true;
    if (!glass) host.appendChild(this.el);
    this.active = false;
    this._move = (e) => this.paint(e);
    this._up = () => this.hide();
  }

  show() {
    this.active = true;
    this.el.hidden = false;
    this.host.addEventListener('pointermove', this._move);
    addEventListener('pointerup', this._up);
    addEventListener('pointercancel', this._up);
  }

  hide() {
    this.active = false;
    this.el.hidden = true;
    this.host.removeEventListener('pointermove', this._move);
    removeEventListener('pointerup', this._up);
    removeEventListener('pointercancel', this._up);
  }

  paint(e) {
    const img = e.target.closest?.('img.flip-image, img.pg, img') || this.host.querySelector('img.pg, img.flip-image');
    if (!(img instanceof HTMLImageElement) || !img.complete) return;
    const r = img.getBoundingClientRect();
    const x = ((e.clientX - r.left) / Math.max(1, r.width)) * 100;
    const y = ((e.clientY - r.top) / Math.max(1, r.height)) * 100;
    this.el.style.left = `${e.clientX}px`;
    this.el.style.top = `${e.clientY}px`;
    this.el.style.backgroundImage = `url(${img.currentSrc || img.src})`;
    this.el.style.backgroundSize = `${r.width * this.scale}px ${r.height * this.scale}px`;
    this.el.style.backgroundPosition = `${x}% ${y}%`;
  }

  toggle() {
    if (this.active) this.hide();
    else this.show();
  }
}

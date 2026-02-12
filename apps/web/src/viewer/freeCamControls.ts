import { PerspectiveCamera, Vector3 } from "three";
import { PointerLockControls } from "three/examples/jsm/controls/PointerLockControls.js";

type KeyState = {
  forward: boolean;
  backward: boolean;
  left: boolean;
  right: boolean;
  up: boolean;
  down: boolean;
};

export class FreeCamControls {
  readonly target = new Vector3();
  enableRotate = true;
  enablePan = true;
  enableZoom = true;

  private readonly camera: PerspectiveCamera;
  private readonly domElement: HTMLElement;
  private readonly controls: PointerLockControls;
  private readonly keyState: KeyState = {
    forward: false,
    backward: false,
    left: false,
    right: false,
    up: false,
    down: false
  };
  private lastUpdate = performance.now();
  private readonly movementSpeed = 12;
  private readonly lookSpeed = 1.75;
  private locked = false;
  private lastCursor = "";

  constructor(camera: PerspectiveCamera, domElement: HTMLElement) {
    this.camera = camera;
    this.domElement = domElement;
    this.controls = new PointerLockControls(camera, domElement);
    this.controls.pointerSpeed = this.lookSpeed;

    this.domElement.addEventListener("mousedown", this.handlePointerLock);
    document.addEventListener("pointerlockchange", this.handlePointerLockChange);
    window.addEventListener("keydown", this.handleKeyDown);
    window.addEventListener("keyup", this.handleKeyUp);
  }

  update(): void {
    const now = performance.now();
    const delta = Math.min((now - this.lastUpdate) / 1000, 0.1);
    this.lastUpdate = now;

    if (!this.enableRotate || !this.locked) {
      return;
    }

    const forward = Number(this.keyState.forward) - Number(this.keyState.backward);
    const right = Number(this.keyState.right) - Number(this.keyState.left);
    const up = Number(this.keyState.up) - Number(this.keyState.down);

    if (forward === 0 && right === 0 && up === 0) {
      return;
    }

    const speed = this.movementSpeed * delta;
    if (right !== 0) {
      this.controls.moveRight(right * speed);
    }
    if (forward !== 0) {
      this.controls.moveForward(forward * speed);
    }
    if (up !== 0) {
      this.camera.position.y += up * speed;
    }
  }

  dispose(): void {
    this.controls.unlock();
    this.controls.dispose();
    this.domElement.removeEventListener("mousedown", this.handlePointerLock);
    document.removeEventListener("pointerlockchange", this.handlePointerLockChange);
    window.removeEventListener("keydown", this.handleKeyDown);
    window.removeEventListener("keyup", this.handleKeyUp);
  }

  setEnabled(enabled: boolean): void {
    this.enableRotate = enabled;
    this.enablePan = enabled;
    this.enableZoom = enabled;
    if (!enabled && this.controls.isLocked) {
      this.controls.unlock();
    }
    if (!enabled) {
      this.restoreCursor();
    }
  }

  private handlePointerLock = (): void => {
    if (!this.enableRotate) {
      return;
    }
    if (!this.controls.isLocked) {
      this.controls.lock();
    }
  };

  private handlePointerLockChange = (): void => {
    this.locked = this.controls.isLocked;
    if (this.locked) {
      this.captureCursor();
    } else {
      this.restoreCursor();
    }
  };

  private captureCursor(): void {
    if (!this.domElement.style.cursor || this.domElement.style.cursor !== "none") {
      this.lastCursor = this.domElement.style.cursor;
      this.domElement.style.cursor = "none";
    }
  }

  private restoreCursor(): void {
    if (this.domElement.style.cursor !== this.lastCursor) {
      this.domElement.style.cursor = this.lastCursor || "";
    }
  }

  private handleKeyDown = (event: KeyboardEvent): void => {
    if (this.isTypingTarget(event.target)) {
      return;
    }
    switch (event.code) {
      case "KeyW":
      case "ArrowUp":
        event.preventDefault();
        this.keyState.forward = true;
        break;
      case "KeyS":
      case "ArrowDown":
        event.preventDefault();
        this.keyState.backward = true;
        break;
      case "KeyA":
      case "ArrowLeft":
        event.preventDefault();
        this.keyState.left = true;
        break;
      case "KeyD":
      case "ArrowRight":
        event.preventDefault();
        this.keyState.right = true;
        break;
      case "Space":
        event.preventDefault();
        this.keyState.up = true;
        break;
      case "ShiftLeft":
      case "ShiftRight":
        event.preventDefault();
        this.keyState.down = true;
        break;
    }
  };

  private handleKeyUp = (event: KeyboardEvent): void => {
    if (this.isTypingTarget(event.target)) {
      return;
    }
    switch (event.code) {
      case "KeyW":
      case "ArrowUp":
        event.preventDefault();
        this.keyState.forward = false;
        break;
      case "KeyS":
      case "ArrowDown":
        event.preventDefault();
        this.keyState.backward = false;
        break;
      case "KeyA":
      case "ArrowLeft":
        event.preventDefault();
        this.keyState.left = false;
        break;
      case "KeyD":
      case "ArrowRight":
        event.preventDefault();
        this.keyState.right = false;
        break;
      case "Space":
        event.preventDefault();
        this.keyState.up = false;
        break;
      case "ShiftLeft":
      case "ShiftRight":
        event.preventDefault();
        this.keyState.down = false;
        break;
    }
  };

  private isTypingTarget(target: EventTarget | null): boolean {
    if (!(target instanceof HTMLElement)) {
      return false;
    }
    const tag = target.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT";
  }
}

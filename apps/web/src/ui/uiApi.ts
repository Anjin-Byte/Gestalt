import type { UiApi } from "../modules/types";

export const createUiApi = (
  container: HTMLElement,
  onChange?: () => void
): UiApi => {
  const values: Record<string, unknown> = {};
  const textNodes = new Map<string, HTMLSpanElement>();

  const clear = () => {
    container.innerHTML = "";
  };

  const addLabel = (labelText: string) => {
    const label = document.createElement("label");
    label.textContent = labelText;
    container.appendChild(label);
    return label;
  };

  const addSlider: UiApi["addSlider"] = ({
    id,
    label,
    min,
    max,
    step,
    initial
  }) => {
    addLabel(label);
    const input = document.createElement("input");
    input.type = "range";
    input.min = String(min);
    input.max = String(max);
    input.step = String(step);
    input.value = String(initial);
    input.dataset.controlId = id;
    values[id] = initial;
    input.addEventListener("input", () => {
      values[id] = Number(input.value);
      onChange?.();
    });
    container.appendChild(input);
  };

  const addNumber: UiApi["addNumber"] = ({
    id,
    label,
    min,
    max,
    step,
    initial
  }) => {
    addLabel(label);
    const input = document.createElement("input");
    input.type = "number";
    input.min = String(min);
    input.max = String(max);
    input.step = String(step);
    input.value = String(initial);
    input.dataset.controlId = id;
    values[id] = initial;
    input.addEventListener("input", () => {
      values[id] = Number(input.value);
      onChange?.();
    });
    container.appendChild(input);
  };

  const addCheckbox: UiApi["addCheckbox"] = ({ id, label, initial }) => {
    const wrapper = document.createElement("div");
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = initial;
    checkbox.dataset.controlId = id;
    values[id] = initial;
    checkbox.addEventListener("change", () => {
      values[id] = checkbox.checked;
      onChange?.();
    });

    const text = document.createElement("span");
    text.textContent = ` ${label}`;
    wrapper.appendChild(checkbox);
    wrapper.appendChild(text);
    container.appendChild(wrapper);
  };

  const addSelect: UiApi["addSelect"] = ({ id, label, options, initial }) => {
    addLabel(label);
    const select = document.createElement("select");
    select.dataset.controlId = id;
    for (const option of options) {
      const item = document.createElement("option");
      item.value = option;
      item.textContent = option;
      select.appendChild(item);
    }
    select.value = initial;
    values[id] = initial;
    select.addEventListener("change", () => {
      values[id] = select.value;
      onChange?.();
    });
    container.appendChild(select);
  };

  const addText: UiApi["addText"] = ({ id, label, initial }) => {
    addLabel(label);
    const value = document.createElement("span");
    value.textContent = initial;
    value.dataset.controlId = id;
    textNodes.set(id, value);
    container.appendChild(value);
  };

  const setText: UiApi["setText"] = (id, value) => {
    const node = textNodes.get(id);
    if (node) {
      node.textContent = value;
    }
  };

  const addFile: UiApi["addFile"] = ({ id, label, accept, onFile }) => {
    addLabel(label);
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept;
    input.dataset.controlId = id;
    input.addEventListener("change", () => {
      const file = input.files?.[0] ?? null;
      const result = onFile(file);
      if (result && typeof (result as Promise<void>).then === "function") {
        (result as Promise<void>).finally(() => onChange?.());
      } else {
        onChange?.();
      }
    });
    container.appendChild(input);
  };

  const addButton: UiApi["addButton"] = ({ label, onClick }) => {
    const button = document.createElement("button");
    button.type = "button";
    button.textContent = label;
    button.addEventListener("click", onClick);
    container.appendChild(button);
  };

  const getValues = () => ({ ...values });

  return {
    addSlider,
    addNumber,
    addCheckbox,
    addSelect,
    addText,
    setText,
    addFile,
    addButton,
    getValues,
    clear
  };
};

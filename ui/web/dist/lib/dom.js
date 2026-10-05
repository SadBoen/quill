// 极小的 DOM 辅助函数 —— 零第三方依赖，手写。
// 只做三件事：建元素、清容器、拼片段。没有虚拟 DOM、没有模板编译。

/**
 * 建一个元素。
 * props 支持：
 *   class   → className
 *   text    → textContent（⚠️ 永远用 textContent，绝不用 innerHTML，避免 XSS）
 *   attrs   → setAttribute 键值对
 *   on      → { click: fn } 事件绑定
 *   其余    → 直接赋值给 node[key]（value / disabled / checked 等）
 * children 会被展平，字符串转成文本节点，null / undefined / false 直接丢弃。
 */
export function h(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, val] of Object.entries(props)) {
    if (val === undefined || val === null || val === false) continue;
    if (key === 'class') node.className = val;
    else if (key === 'text') node.textContent = String(val);
    else if (key === 'attrs') {
      for (const [a, av] of Object.entries(val)) {
        if (av === undefined || av === null || av === false) continue;
        node.setAttribute(a, av === true ? '' : String(av));
      }
    } else if (key === 'on') {
      for (const [ev, fn] of Object.entries(val)) node.addEventListener(ev, fn);
    } else {
      node[key] = val;
    }
  }
  for (const child of [].concat(children)) {
    if (child === undefined || child === null || child === false || child === '') continue;
    node.appendChild(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/** 清空容器（逐个删子节点，不留空白文本）。 */
export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
}

/** 把一组节点塞进容器并替换旧内容。 */
export function mount(node, children) {
  clear(node);
  for (const child of [].concat(children)) {
    if (child === undefined || child === null || child === false) continue;
    node.appendChild(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return node;
}

/** 把 API 错误渲染成一块显眼的面板：说明 + 下一步，两段都必须显示。 */
export function errorPanel(err) {
  return h('div', { class: 'panel panel-error' }, [
    h('div', { class: 'panel-title', text: '请求失败' }),
    h('p', { class: 'panel-detail', text: err.detail }),
    h('div', { class: 'panel-next' }, [
      h('span', { class: 'panel-next-label', text: '下一步该做什么' }),
      h('p', { class: 'panel-next-body', text: err.nextStep }),
    ]),
    err.code ? h('p', { class: 'panel-code', text: '错误码：' + err.code + '（HTTP ' + err.status + '）' }) : null,
  ]);
}

/** loading 态。传 label 说明正在做什么。 */
export function loadingPanel(label) {
  return h('div', { class: 'panel panel-loading' }, [
    h('span', { class: 'spinner', attrs: { 'aria-hidden': 'true' } }),
    h('span', { text: label || '正在加载…' }),
  ]);
}

/** 空状态：没有数据时说明「为什么空」和「怎么让它有数据」。 */
export function emptyPanel(title, hint) {
  return h('div', { class: 'panel panel-empty' }, [
    h('div', { class: 'panel-title', text: title }),
    hint ? h('p', { class: 'panel-detail', text: hint }) : null,
  ]);
}

/** 一条成功/提示信息。tone ∈ ok / warn / info */
export function notePanel(tone, title, body) {
  return h('div', { class: 'panel panel-' + tone }, [
    h('div', { class: 'panel-title', text: title }),
    body ? h('p', { class: 'panel-detail', text: body }) : null,
  ]);
}

/** 把「说明 + 下一步」渲染成一块可读的面板（用于前端自造的失败）。 */
export function plainError(title, detail, nextStep) {
  return errorPanel({ code: '', status: 0, detail: detail, nextStep: nextStep, title: title });
}

/** 只读 JSON 折叠块 —— 派工这类形状不确定的数据用它兜底展示。 */
export function jsonDetails(summary, value) {
  return h('details', { class: 'json-details' }, [
    h('summary', { text: summary }),
    h('pre', { class: 'json-body', text: JSON.stringify(value, null, 2) }),
  ]);
}

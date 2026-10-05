// 总览页 —— 只读 /healthz。免鉴权，任何时候可查。

import { api } from '../lib/api.js';
import { h, mount, loadingPanel, errorPanel, notePanel, jsonDetails } from '../lib/dom.js';

export function render(root, ctx) {
  mount(root, [
    h('div', { class: 'page-head' }, [
      h('h2', { class: 'page-title', text: '总览' }),
      h('button', {
        class: 'btn',
        type: 'button',
        text: '重新检查',
        on: { click: () => load() },
      }),
    ]),
    h('div', { id: 'overview-body' }, [loadingPanel('正在读取服务状态…')]),
  ]);
  load();

  async function load() {
    const body = root.querySelector('#overview-body');
    mount(body, [loadingPanel('正在读取服务状态…')]);
    let health;
    try {
      health = await api.health();
    } catch (err) {
      mount(body, [errorPanel(err), offlineHint()]);
      ctx.reportBackendDown(true);
      return;
    }
    ctx.reportBackendDown(false);
    mount(body, [renderHealth(health)]);
  }
}

function offlineHint() {
  return notePanel(
    'warn',
    '总览拿不到数据',
    '这是因为服务没响应。其余页面也会同样失败。先把后端跑起来，再点「重新检查」。'
  );
}

function renderHealth(health) {
  const blocks = [];
  const storage = health.storage || {};

  // ⚠️ 存储未就绪要显眼：红色横幅 + 完整文字，不能折叠成一行小字。
  if (storage.ready === false) {
    blocks.push(
      h('div', { class: 'banner banner-error' }, [
        h('div', { class: 'banner-title', text: '存储未就绪 —— 数据相关的功能现在都用不了' }),
        h('p', { class: 'banner-body', text: storage.detail || '后端没有给出具体原因。' }),
        storage.db_path ? h('p', { class: 'banner-body', text: '数据库路径：' + storage.db_path }) : null,
        Array.isArray(storage.missing_tables) && storage.missing_tables.length
          ? h('div', { class: 'banner-body' }, [
              h('span', { text: '缺少数据表：' }),
              h('ul', { class: 'plain-list' }, storage.missing_tables.map((t) => h('li', { text: t }))),
            ])
          : null,
        h('p', { class: 'banner-body', text: '下一步该做什么：先执行 quill doctor --section=db 打印数据库诊断；若提示缺表，按 crates/quill-store/migrations/0001_init.sql 执行迁移后重启服务；若提示打不开数据库，用 QUILL_DB_PATH 指向一个可写路径后重启。' }),
      ])
    );
  } else {
    blocks.push(
      notePanel('ok', '存储就绪', '数据库可打开且数据表齐全。')
    );
  }

  // ⚠️ warnings 非空也要显眼。
  const warnings = Array.isArray(health.warnings) ? health.warnings : [];
  if (warnings.length > 0) {
    blocks.push(
      h('div', { class: 'banner banner-warn' }, [
        h('div', { class: 'banner-title', text: '配置有 ' + warnings.length + ' 条告警（服务已回退默认值并继续运行）' }),
        h('ul', { class: 'plain-list' }, warnings.map((w) => h('li', {
          text: '[' + (w.source || '未标注来源') + '] ' + (w.message || '（没有文字说明）'),
        }))),
        h('p', { class: 'banner-body', text: '下一步该做什么：按上面每条的说明修正对应环境变量后重启服务；在此之前功能可能不完整，但不会静默失败。' }),
      ])
    );
  }

  if (health.ui_assets_available === false) {
    blocks.push(
      h('div', { class: 'banner banner-error' }, [
        h('div', { class: 'banner-title', text: '前端资源目录不可用' }),
        h('p', { class: 'banner-body', text: 'QUILL_WEB_DIR 指向的目录不存在或为空，界面用的是内嵌兜底资源。' }),
        h('p', { class: 'banner-body', text: '下一步该做什么：确认 ui/web/dist 目录已随服务部署，并把 QUILL_WEB_DIR 指向它，然后重启服务。' }),
      ])
    );
  }

  const facts = [
    ['服务状态', health.status || '（后端未给出）'],
    ['版本', health.version || '（后端未给出）'],
    ['监听地址', health.addr || '（后端未给出）'],
    ['前端资源', health.ui_assets_available ? '可用' : '不可用'],
    ['存储', storage.ready ? '就绪' : '未就绪'],
    ['数据库路径', storage.db_path || '（未装配）'],
  ];

  blocks.push(
    h('table', { class: 'kv' }, [
      h('tbody', {}, facts.map((row) => h('tr', {}, [
        h('th', { attrs: { scope: 'row' }, text: row[0] }),
        h('td', { text: row[1] }),
      ]))),
    ])
  );

  blocks.push(jsonDetails('查看 /healthz 原始响应', health));
  return blocks;
}

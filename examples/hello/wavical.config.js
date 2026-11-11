export default {
  appId: 'wavical-hello',
  window: {
    title: 'Wavical — Hello World',
    width: 1200,
    height: 800
  },
  ai: {
    enabled: true,
    mode: 'local',
    local: {
      chatModel: 'qwen2.5-1.5b-instruct',
      maxTokens: 512,
      idleMinutes: 10,
    }
  },
  capabilities: {
    'core.ping': true,
    'clipboard.read': true,
    'clipboard.write': true,
    'notify.send': true,
    'dialog.openFile': true,
    'dialog.saveFile': true,
    'fs.readGranted': true,
    'fs.writeGranted': true,
    'tray.set': true,
    'menu.showContextMenu': true,
    'shortcut.register': true,
    'badge.set': true,
    'ai.chat': true,
    'backend.invoke': true
  },
  backend: {
    enabled: true,
    runtime: 'node',
    version: '22.11.0',
    allowPatchUpdates: false,
    entry: 'backend/main.js',
    methods: ['hello.greet'],
    timeoutSeconds: 30,
    permissions: {
      fsRead: [],
      fsWrite: [],
      childProcess: false,
      workers: false
    },
    env: [],
    unrestricted: false
  },
  shortcuts: [
    'Ctrl+Shift+K'
  ],
  fileAssociations: [
    'wavhello'
  ],
  tray: {
    icon: 'icon.png'
  },
  dialog: {
    filters: {
      text: { name: 'Text Documents', extensions: ['txt', 'md', 'json'] },
      all: { name: 'All Files', extensions: ['*'] }
    }
  }
};

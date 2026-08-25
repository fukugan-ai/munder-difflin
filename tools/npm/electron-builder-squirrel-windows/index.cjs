'use strict';

class UnsupportedSquirrelWindowsTarget {
  constructor() {
    throw new Error(
      'Squirrel.Windows is unsupported; use the configured NSIS or portable Windows target.'
    );
  }
}

module.exports = { default: UnsupportedSquirrelWindowsTarget };

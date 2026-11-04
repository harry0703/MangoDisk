/*
 * MangoDisk legacy-browser polyfills.
 *
 * macOS 10.13 (High Sierra) ships Safari 11 (2017 WebKit). The application's
 * modern JavaScript dependencies rely on runtime methods that Safari 11 does
 * not provide. esbuild/rolldown only lower syntax to the configured target;
 * they do not inject these runtime methods, so we install minimal, guarded
 * replacements before the application bundle executes.
 *
 * Every patch is guarded by an existence check, so modern macOS versions keep
 * their native implementations untouched. The script is intentionally written
 * in conservative syntax so it can parse and run on the oldest supported
 * WebKit.
 *
 * Loaded as a classic <script> (not type="module") from every HTML entry so it
 * executes synchronously before the deferred application bundle.
 */
(function () {
  'use strict';

  var globalScope = typeof globalThis !== 'undefined'
    ? globalThis
    : (typeof window !== 'undefined' ? window : this);

  function definePolyfill(target, name, value) {
    var current = target[name];
    if (current !== undefined && current !== null) {
      return;
    }
    Object.defineProperty(target, name, {
      value: value,
      writable: true,
      configurable: true,
      enumerable: false
    });
  }

  function flattenInto(target, source, depth) {
    var length = source.length;
    for (var i = 0; i < length; i += 1) {
      var item = source[i];
      if (Array.isArray(item) && depth > 0) {
        flattenInto(target, item, depth - 1);
      } else {
        target.push(item);
      }
    }
    return target;
  }

  definePolyfill(Array.prototype, 'flat', function flat(depth) {
    var result = [];
    var level = depth === undefined ? 1 : depth;
    flattenInto(result, this, level);
    return result;
  });

  definePolyfill(Array.prototype, 'flatMap', function flatMap(mapper, thisArg) {
    var result = [];
    for (var i = 0; i < this.length; i += 1) {
      var mapped = mapper.call(thisArg, this[i], i, this);
      if (Array.isArray(mapped)) {
        flattenInto(result, mapped, 1);
      } else {
        result.push(mapped);
      }
    }
    return result;
  });

  definePolyfill(Array.prototype, 'at', function at(offset) {
    var length = this.length;
    var index = offset < 0 ? length + offset : offset;
    return this[index];
  });

  definePolyfill(Array.prototype, 'findLast', function findLast(predicate, thisArg) {
    for (var i = this.length - 1; i >= 0; i -= 1) {
      if (predicate.call(thisArg, this[i], i, this)) {
        return this[i];
      }
    }
    return undefined;
  });

  definePolyfill(Array.prototype, 'findLastIndex', function findLastIndex(predicate, thisArg) {
    for (var i = this.length - 1; i >= 0; i -= 1) {
      if (predicate.call(thisArg, this[i], i, this)) {
        return i;
      }
    }
    return -1;
  });

  definePolyfill(Object, 'fromEntries', function fromEntries(iterable) {
    var result = {};
    var iterator = iterable[Symbol.iterator]();
    var step = iterator.next();
    while (!step.done) {
      var entry = step.value;
      result[entry[0]] = entry[1];
      step = iterator.next();
    }
    return result;
  });

  definePolyfill(String.prototype, 'replaceAll', function replaceAll(pattern, replacement) {
    var string = String(this);
    if (pattern instanceof RegExp) {
      if (!pattern.global) {
        var flags = 'g' + (pattern.ignoreCase ? 'i' : '') + (pattern.multiline ? 'm' : '');
        pattern = new RegExp(pattern.source, flags);
      }
      return string.replace(pattern, replacement);
    }
    var target = String(pattern).replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    return string.replace(new RegExp(target, 'g'), replacement);
  });

  function randomHex(byteCount) {
    var cryptoObj = typeof crypto !== 'undefined' ? crypto : null;
    var bytes = new Uint8Array(byteCount);
    if (cryptoObj && typeof cryptoObj.getRandomValues === 'function') {
      cryptoObj.getRandomValues(bytes);
    } else {
      for (var i = 0; i < byteCount; i += 1) {
        bytes[i] = Math.floor(Math.random() * 256);
      }
    }
    var hex = '';
    for (var j = 0; j < byteCount; j += 1) {
      var byte = bytes[j];
      hex += byte.toString(16).padStart(2, '0');
    }
    return hex;
  }

  function fallbackRandomUUID() {
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function (character) {
      var random = Math.floor(Math.random() * 16);
      var value = character === 'x' ? random : (random & 0x3) | 0x8;
      return value.toString(16);
    });
  }

  var cryptoObj = typeof crypto !== 'undefined' ? crypto : null;
  if (cryptoObj && typeof cryptoObj.randomUUID !== 'function') {
    cryptoObj.randomUUID = fallbackRandomUUID;
  }

  function deepClone(value) {
    if (value === null || typeof value !== 'object') {
      return value;
    }
    if (Array.isArray(value)) {
      var arrayResult = [];
      for (var i = 0; i < value.length; i += 1) {
        arrayResult[i] = deepClone(value[i]);
      }
      return arrayResult;
    }
    if (value instanceof Date) {
      return new Date(value.getTime());
    }
    if (value instanceof RegExp) {
      var flags = '';
      if (value.global) {
        flags += 'g';
      }
      if (value.ignoreCase) {
        flags += 'i';
      }
      if (value.multiline) {
        flags += 'm';
      }
      return new RegExp(value.source, flags);
    }
    if (typeof Map !== 'undefined' && value instanceof Map) {
      var mapResult = new Map();
      value.forEach(function (mapValue, mapKey) {
        mapResult.set(deepClone(mapKey), deepClone(mapValue));
      });
      return mapResult;
    }
    if (typeof Set !== 'undefined' && value instanceof Set) {
      var setResult = new Set();
      value.forEach(function (setValue) {
        setResult.add(deepClone(setValue));
      });
      return setResult;
    }
    var objectResult = {};
    var keys = Object.keys(value);
    for (var j = 0; j < keys.length; j += 1) {
      objectResult[keys[j]] = deepClone(value[keys[j]]);
    }
    return objectResult;
  }

  definePolyfill(globalScope, 'structuredClone', deepClone);
})();

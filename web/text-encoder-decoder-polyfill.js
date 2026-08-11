// Minimal TextDecoder/TextEncoder for AudioWorkletGlobalScope
var _TDPolyfill = function(label, options) {
    this._fatal = options && options.fatal;
};
_TDPolyfill.prototype.decode = function(bytes) {
    if (!bytes) return '';
    var r = '', i = 0;
    while (i < bytes.length) {
        var b1 = bytes[i++];
        if (b1 < 0x80) { r += String.fromCharCode(b1); }
        else if (b1 < 0xC0) { r += '\uFFFD'; }
        else if (b1 < 0xE0) { r += String.fromCharCode(((b1&0x1F)<<6)|((bytes[i++])&0x3F)); }
        else if (b1 < 0xF0) { r += String.fromCharCode(((b1&0x0F)<<12)|((bytes[i++])&0x3F)<<6|((bytes[i++])&0x3F)); }
        else { var c=((b1&7)<<18)|((bytes[i++]&0x3F)<<12)|((bytes[i++]&0x3F)<<6)|(bytes[i++]&0x3F); c-=0x10000; r+=String.fromCharCode(0xD800+(c>>10),0xDC00+(c&0x3FF)); }
    }
    return r;
};
var _TEPolyfill = function() {};
_TEPolyfill.prototype.encode = function(s) {
    if (!s) return new Uint8Array(0);
    var b = [];
    for (var i = 0; i < s.length; i++) {
        var c = s.charCodeAt(i);
        if (c >= 0xD800 && c <= 0xDBFF && i+1 < s.length) { c = 0x10000+((c-0xD800)<<10)+(s.charCodeAt(++i)-0xDC00); }
        if (c < 0x80) { b.push(c); }
        else if (c < 0x800) { b.push(0xC0|(c>>6), 0x80|(c&0x3F)); }
        else if (c < 0x10000) { b.push(0xE0|(c>>12), 0x80|((c>>6)&0x3F), 0x80|(c&0x3F)); }
        else { b.push(0xF0|(c>>18), 0x80|((c>>12)&0x3F), 0x80|((c>>6)&0x3F), 0x80|(c&0x3F)); }
    }
    return new Uint8Array(b);
};
_TEPolyfill.prototype.encodeInto = function(s, view) {
    var e = this.encode(s), n = Math.min(e.length, view.length);
    for (var i = 0; i < n; i++) view[i] = e[i];
    return { read: s.length, written: n };
};
try {
    if (typeof globalThis.TextDecoder === 'undefined') globalThis.TextDecoder = _TDPolyfill;
    if (typeof globalThis.TextEncoder === 'undefined') globalThis.TextEncoder = _TEPolyfill;
} catch(e) {}

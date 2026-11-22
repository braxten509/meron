// Builds the script used by the mobile HTML mail reader.
// The web view reports document dimensions and persists quote changes.
// Layout, colors and image placement are calculated inside the rendered document.

package jp.nonbili.meron.ui

internal fun mailDocumentScript(
    fitWideContent: Boolean,
    initialOverflow: Pair<Int, Int>,
    showQuotedLabel: String,
    hideQuotedLabel: String,
    quoteOpen: Boolean,
    floorChannels: String,
): String =
    """
                  // Feed/newsletter HTML often lists photos as a bare run of
                  // sibling `<img>` (or single-image `<p>`/`<div>`) elements,
                  // which would otherwise stack one per row at full width.
                  // Wrap runs of 2+ into a flex grid so they tile 2-3 across.
                  function isImageOnlyBlock(el) {
                    if (!el || el.nodeType !== 1) return false;
                    if (el.tagName === 'IMG') return true;
                    if (el.children.length !== 1) return false;
                    for (var i = 0; i < el.childNodes.length; i++) {
                      var n = el.childNodes[i];
                      if (n.nodeType === 3 && n.textContent.trim().length > 0) return false;
                    }
                    return isImageOnlyBlock(el.children[0]);
                  }
                  function findImageBlock(img) {
                    var node = img;
                    while (node.parentElement && node.parentElement !== document.body) {
                      if (isImageOnlyBlock(node.parentElement)) {
                        node = node.parentElement;
                      } else {
                        break;
                      }
                    }
                    return node;
                  }
                  function groupConsecutiveImages() {
                    var imgs = Array.prototype.slice.call(document.querySelectorAll('img'));
                    var blocks = [];
                    var seen = [];
                    imgs.forEach(function (img) {
                      var block = findImageBlock(img);
                      if (seen.indexOf(block) === -1) {
                        seen.push(block);
                        blocks.push(block);
                      }
                    });
                    var i = 0;
                    while (i < blocks.length) {
                      var run = [blocks[i]];
                      var j = i + 1;
                      while (
                        j < blocks.length &&
                        run[run.length - 1].nextElementSibling === blocks[j] &&
                        run[run.length - 1].parentElement === blocks[j].parentElement
                      ) {
                        run.push(blocks[j]);
                        j++;
                      }
                      if (run.length > 1) {
                        var grid = document.createElement('div');
                        grid.setAttribute('data-meron-image-grid', '1');
                        run[0].parentNode.insertBefore(grid, run[0]);
                        run.forEach(function (block) {
                          grid.appendChild(block);
                        });
                      }
                      i = j;
                    }
                  }
                  // Mail bodies arrive as whole documents and routinely carry
                  // their own <meta name="viewport">, which lands in our body
                  // and, being later in the document, is the one Blink honours.
                  // A single stray `content="target-densitydpi=device-dpi"` --
                  // common in mail templates, and what Gemini's welcome mail
                  // ships -- declares no width at all, so with useWideViewPort
                  // on the layout falls back to the 980px desktop default and
                  // the whole page renders at ~0.37 scale: legible mail shrunk
                  // to nothing. Dropping every viewport but ours puts the width
                  // back under our control, which is also what applyWidthFit
                  // below assumes when it measures and rewrites.
                  function dropForeignViewports() {
                    var metas = document.querySelectorAll('meta[name="viewport"]');
                    var ours = null;
                    for (var i = 0; i < metas.length; i++) {
                      if (metas[i].id === 'meron-viewport') {
                        ours = metas[i];
                      } else if (metas[i].parentNode) {
                        metas[i].parentNode.removeChild(metas[i]);
                      }
                    }
                    // Removing a meta does not make Blink recompute the viewport
                    // -- the mail's description stays in effect over an empty
                    // head -- but writing to one does. Re-asserting the same
                    // content is what actually applies the width above.
                    if (ours) ours.setAttribute('content', ours.getAttribute('content'));
                  }
                  // The CSS overrides reflow most mail into the view. What they
                  // cannot shrink -- a wide <pre>, an oversized image, a fixed
                  // width in an inline style rather than the width attribute --
                  // would otherwise be clipped outright, because the view is
                  // sized to its content and never scrolls sideways. Widening
                  // the viewport to the content's natural width instead makes
                  // WebView scale the whole page down to fit.
                  var fitWide = ${if (fitWideContent) "true" else "false"};
                  var fitScale = 1;
                  var viewWidth = 0;
                  function visibleWidth() {
                    return (
                      (window.visualViewport && window.visualViewport.width) ||
                      window.innerWidth ||
                      0
                    );
                  }
                  function applyWidthFit() {
                    // Pin once: after the viewport widens, the page is no longer
                    // overflowing, so re-measuring would just undo the fit.
                    if (!fitWide || fitScale !== 1) return;
                    // By id, not by name: a mail's own viewport meta would win
                    // over ours in Blink, so rewriting the first match could
                    // rewrite a tag the engine is already ignoring.
                    var meta = document.getElementById('meron-viewport');
                    if (!meta) return;
                    // Captured before the viewport widens, so it stays the view's
                    // width in dp -- the denominator the height bridge needs.
                    if (!viewWidth) viewWidth = visibleWidth();
                    if (!viewWidth) return;
                    var natural = Math.max(
                      document.documentElement.scrollWidth || 0,
                      document.body ? document.body.scrollWidth : 0
                    );
                    // A little slack: sub-pixel table borders routinely round up
                    // and are not worth shrinking the whole page for.
                    if (natural <= viewWidth + 2) return;
                    // Widening the viewport alone does not rescale a page that has
                    // already loaded: the layout grows to the natural width but
                    // the scale stays 1, leaving the mail sideways-scrollable. So
                    // the fit scale is stated outright.
                    var width = Math.ceil(natural);
                    fitScale = viewWidth / width;
                    meta.setAttribute(
                      'content',
                      'width=' + width + ', initial-scale=' + fitScale + ', minimum-scale=' + fitScale
                    );
                  }
                  // The root element is the one that scrolls, so its scrollHeight
                  // never drops below the view's own height -- sizing off it pins
                  // a short mail to whatever height the view was first laid out
                  // at and lets the measurement only ever grow, padding a
                  // one-line message out with blank space. Measure the boxes,
                  // whose heights are auto and so track the content, and keep
                  // scrollHeight purely as an overflow signal for content that
                  // escapes the body (an absolutely positioned block whose
                  // containing block is the initial one reaches nothing else).
                  //
                  // That signal only says anything while the view is still
                  // shorter than the content: once it grows to fit, scrollHeight
                  // and clientHeight agree again and the reading is
                  // indistinguishable from an empty document. So an extent, once
                  // seen, is carried as a floor for the rest of this document --
                  // re-deriving it each pass is what would make the view
                  // oscillate between the overflow height and the empty box.
                  //
                  // A floor is only worth carrying for a scroll area the boxes
                  // cannot account for. A tall ordinary mail overflows the
                  // view's first layout too, and retaining that would pin it
                  // there: text that reflows shorter once the view settles at
                  // its width could never give the height back.
                  //
                  // The desktop frame applies the same rules; its arithmetic
                  // (and the tests pinning it) lives in chat/frameHeight.ts.
                  //
                  // A list item that scrolled out and back in gets a fresh
                  // document in a view already sized to the saved height, where
                  // the overflow fits and so never shows: the floor it had found
                  // is handed back, or the escaping content reads as the empty
                  // box and the item collapses before growing back. Only at the
                  // width it was found at, checked on the first measurement:
                  // narrower or wider, the content runs to a different length.
                  var overflowExtent = 0;
                  var seededOverflowExtent = ${initialOverflow.first};
                  var seededOverflowWidth = ${initialOverflow.second};
                  var layoutWidth = 0;
                  var reportedOverflowExtent = 0;
                  function contentHeight() {
                    var root = document.documentElement;
                    var body = document.body;
                    var h = 0;
                    if (body) {
                      var rect = body.getBoundingClientRect();
                      h = Math.max(h, rect.top + rect.height, rect.top + body.scrollHeight);
                    }
                    if (root) {
                      var rootRect = root.getBoundingClientRect();
                      h = Math.max(h, rootRect.top + rootRect.height);
                      if (
                        root.scrollHeight > root.clientHeight + 1 &&
                        root.scrollHeight > h + 1
                      ) {
                        overflowExtent = Math.max(overflowExtent, root.scrollHeight);
                      }
                    }
                    return Math.max(h, overflowExtent);
                  }
                  // What a document of plain flowing text needs: the body laid out
                  // at max-content, read and put back within the same task so
                  // nothing paints it. Anything whose layout follows the width it
                  // is given (tables, pictures, fixed widths) reports -1, and the
                  // bubble stays at full width for it.
                  function naturalWidth() {
                    var body = document.body;
                    if (!body || body.querySelector('table, img, video, iframe, svg, pre, [width], [style*="width"]')) return -1;
                    var style = body.style;
                    var w = [style.getPropertyValue('width'), style.getPropertyPriority('width')];
                    var mw = [style.getPropertyValue('max-width'), style.getPropertyPriority('max-width')];
                    style.setProperty('width', 'max-content', 'important');
                    style.setProperty('max-width', 'none', 'important');
                    var natural = Math.ceil(body.getBoundingClientRect().width);
                    if (w[0]) style.setProperty('width', w[0], w[1]); else style.removeProperty('width');
                    if (mw[0]) style.setProperty('max-width', mw[0], mw[1]); else style.removeProperty('max-width');
                    return natural > 0 ? Math.ceil(natural * fitScale) : -1;
                  }
                  // How far the reader has pinched in, where the view allows it.
                  // Taken from the two viewports rather than visualViewport.scale:
                  // the fit above already sets a page scale, and their ratio is 1
                  // at whatever scale the document rests at. The view is sized
                  // to its content and never scrolls down, so the height it is
                  // given has to grow with the zoom or the enlarged mail would
                  // be cut off at its unzoomed length.
                  var pinchZooms = ${if (MailWebViewPinchZooms) "true" else "false"};
                  function pinchZoom() {
                    var vv = window.visualViewport;
                    var layout = document.documentElement ? document.documentElement.clientWidth : 0;
                    if (!pinchZooms || !vv || !vv.width || !layout) return 1;
                    var zoom = layout / vv.width;
                    // Rounding in either width is not a zoom.
                    return zoom > 1.01 ? zoom : 1;
                  }
                  function report() {
                    // A lazy list can attach a fresh web view and load it before
                    // the view is laid out. At zero width every word wraps onto a
                    // line of its own, and a mail that needs 8000px reads as
                    // hundreds of thousands: reported, that sizes the list item
                    // off into the distance and the list jumps. The resize
                    // observer reports again once the view has its width.
                    if (!visibleWidth()) return;
                    if (!layoutWidth) {
                      // Before the fit pass, which widens the viewport.
                      layoutWidth = Math.round(visibleWidth());
                      if (seededOverflowWidth === layoutWidth) overflowExtent = seededOverflowExtent;
                      reportedOverflowExtent = overflowExtent;
                    }
                    applyWidthFit();
                    var nw = naturalWidth();
                    if (window.MeronWidth && window.MeronWidth.report) {
                      window.MeronWidth.report(nw);
                    } else if (
                      window.webkit &&
                      window.webkit.messageHandlers &&
                      window.webkit.messageHandlers.meronWidth
                    ) {
                      window.webkit.messageHandlers.meronWidth.postMessage(nw);
                    }
                    // The measurement is in the (possibly widened) layout
                    // viewport's CSS pixels; the view renders it at fitScale, so
                    // scale it back to dp or the view gets sized to a phantom tail.
                    var h = Math.ceil(contentHeight() * fitScale * pinchZoom());
                    if (overflowExtent !== reportedOverflowExtent) {
                      reportedOverflowExtent = overflowExtent;
                      if (window.MeronOverflow && window.MeronOverflow.report) {
                        window.MeronOverflow.report(overflowExtent, layoutWidth);
                      } else if (
                        window.webkit &&
                        window.webkit.messageHandlers &&
                        window.webkit.messageHandlers.meronOverflow
                      ) {
                        window.webkit.messageHandlers.meronOverflow.postMessage([overflowExtent, layoutWidth]);
                      }
                    }
                    if (window.MeronHeight && window.MeronHeight.report) {
                      window.MeronHeight.report(h);
                    } else if (
                      window.webkit &&
                      window.webkit.messageHandlers &&
                      window.webkit.messageHandlers.meronHeight
                    ) {
                      window.webkit.messageHandlers.meronHeight.postMessage(h);
                    }
                  }
                  document.addEventListener('click', function (event) {
                    var target = event.target;
                    var image = target && target.closest ? target.closest('img[src]') : null;
                    if (image) {
                      var src = image.getAttribute('src');
                      if (src) {
                        event.preventDefault();
                        if (window.MeronImage && window.MeronImage.open) {
                          window.MeronImage.open(src);
                        } else if (
                          window.webkit &&
                          window.webkit.messageHandlers &&
                          window.webkit.messageHandlers.meronImage
                        ) {
                          window.webkit.messageHandlers.meronImage.postMessage(src);
                        }
                        return;
                      }
                    }
                    var anchor = target && target.closest ? target.closest('a[href]') : null;
                    if (!anchor) return;
                    var href = anchor.getAttribute('href');
                    if (!href || href.charAt(0) === '#') return;
                    event.preventDefault();
                    var url = anchor.href || href;
                    if (window.MeronLink && window.MeronLink.open) {
                      window.MeronLink.open(url);
                    } else if (
                      window.webkit &&
                      window.webkit.messageHandlers &&
                      window.webkit.messageHandlers.meronLink
                    ) {
                      window.webkit.messageHandlers.meronLink.postMessage(url);
                    }
                  });
                  // Put the "•••" toggle where the core-marked quote starts. The
                  // root already carries the folded class; opening it grows the
                  // document, which the ResizeObserver below reports.
                  var showQuotedLabel = ${jsStringLiteral(showQuotedLabel)};
                  var hideQuotedLabel = ${jsStringLiteral(hideQuotedLabel)};
                  var quoteInitiallyOpen = $quoteOpen;
                  function reportQuoteToggle(open) {
                    if (window.MeronQuote && window.MeronQuote.toggle) {
                      window.MeronQuote.toggle(open);
                    } else if (
                      window.webkit &&
                      window.webkit.messageHandlers &&
                      window.webkit.messageHandlers.meronQuote
                    ) {
                      window.webkit.messageHandlers.meronQuote.postMessage(open);
                    }
                  }
                  function installQuoteFold() {
                    var first = document.querySelector('[data-meron-quote]');
                    if (!first || !first.parentNode) return;
                    var root = document.documentElement;
                    var toggle = document.createElement('button');
                    toggle.type = 'button';
                    toggle.className = 'meron-quote-toggle';
                    first.parentNode.insertBefore(toggle, first);
                    function apply(open) {
                      root.classList.toggle('meron-quote-folded', !open);
                      toggle.setAttribute('aria-label', open ? hideQuotedLabel : showQuotedLabel);
                      toggle.setAttribute('aria-expanded', open ? 'true' : 'false');
                    }
                    toggle.addEventListener('click', function (event) {
                      event.preventDefault();
                      event.stopPropagation();
                      var open = root.classList.contains('meron-quote-folded');
                      apply(open);
                      reportQuoteToggle(open);
                      report();
                    });
                    apply(quoteInitiallyOpen);
                  }
                  installQuoteFold();
                  // Before the first report: every width the script measures,
                  // and the view height derived from them, is read under the
                  // viewport this leaves in place.
                  // Dark bodies (see the meron-dark rules): a mail designed
                  // dark already -- a dark page, or light text written for a
                  // dark client -- would come out bright if inverted, so it
                  // keeps its own colors. Judged the way the desktop frames
                  // do: the page color the core hoisted or the body paints,
                  // else the tone most of the visible text is drawn in.
                  function colorParts(value) {
                    var match = /^rgba?\(([^)]*)\)/.exec((value || '').trim());
                    if (!match) return null;
                    var parts = match[1].split(/[\s,\/]+/).filter(Boolean).map(parseFloat);
                    return parts.length < 3 ? null : parts;
                  }
                  function colorTone(value) {
                    var parts = colorParts(value);
                    if (!parts || (parts.length > 3 && parts[3] < 0.6)) return null;
                    var lum = (0.2126 * parts[0] + 0.7152 * parts[1] + 0.0722 * parts[2]) / 255;
                    return lum > 0.55 ? 'light' : 'dark';
                  }
                  function bodyMeta(name) {
                    var meta = document.querySelector('meta[name="' + name + '"]');
                    return meta ? (meta.getAttribute('content') || '').trim() : '';
                  }
                  function canvasTone() {
                    var probe = document.createElement('span');
                    var declared = bodyMeta('meron-body-bg');
                    if (declared) {
                      probe.style.color = declared;
                      document.body.appendChild(probe);
                      var tone = colorTone(getComputedStyle(probe).color);
                      probe.remove();
                      if (tone) return tone;
                    }
                    return colorTone(getComputedStyle(document.body).backgroundColor);
                  }
                  function textTone() {
                    var light = 0;
                    var dark = 0;
                    (function walk(el) {
                      if (/^(STYLE|SCRIPT|BUTTON)$/.test(el.tagName)) return;
                      var style = getComputedStyle(el);
                      if (style.display === 'none' || style.visibility === 'hidden') return;
                      if (style.opacity === '0' || style.fontSize === '0px') return;
                      var tone = colorTone(style.color);
                      for (var i = 0; i < el.childNodes.length; i++) {
                        var node = el.childNodes[i];
                        if (node.nodeType === 1) {
                          walk(node);
                        } else if (node.nodeType === 3 && tone) {
                          var length = node.textContent.trim().length;
                          if (tone === 'light') light += length;
                          else dark += length;
                        }
                      }
                    })(document.body);
                    if (!light && !dark) return null;
                    return light > dark ? 'light' : 'dark';
                  }
                  // A picture drawn as a background is turned back to its own
                  // colors with the images -- but only where it is the whole
                  // of the element: turning a box back also turns back the
                  // text in it, which then reads as sent on a canvas that no
                  // longer is.
                  function markBackgroundPictures() {
                    var all = document.body.getElementsByTagName('*');
                    for (var i = 0; i < all.length; i++) {
                      var el = all[i];
                      if (getComputedStyle(el).backgroundImage.indexOf('url(') === -1) continue;
                      if (el.textContent.trim()) continue;
                      el.setAttribute('data-meron-picture', '');
                    }
                  }
                  // The layer over the body (see the meron-dark rules) caps
                  // what is in the flow, but a positioned box paints above that
                  // layer, and so does everything inside one: its white would
                  // invert to black, not to the surface. Such a background is
                  // held to the floor on the element itself; the pictures are
                  // turned back to their own colors, which a cap would then
                  // show through.
                  var darkFloor = [$floorChannels];
                  function capBackgrounds() {
                    var all = document.body.querySelectorAll('*');
                    // Written once the walk has read every element: a write in
                    // between would have the engine resolve the document's
                    // styles again for the next read.
                    var capped = [];
                    for (var i = 0; i < all.length; i++) {
                      var el = all[i];
                      if (el.matches('img, video, [data-meron-picture], [data-meron-picture] *')) continue;
                      var color = colorParts(getComputedStyle(el).backgroundColor);
                      if (!color || color[3] === 0) continue;
                      if (color[0] <= darkFloor[0] && color[1] <= darkFloor[1] && color[2] <= darkFloor[2]) continue;
                      var channels = [0, 1, 2].map(function (c) { return Math.min(color[c], darkFloor[c]); });
                      capped.push([el, 'rgba(' + channels.join(', ') + ', ' + (color.length > 3 ? color[3] : 1) + ')']);
                    }
                    capped.forEach(function (pair) {
                      pair[0].style.setProperty('background-color', pair[1], 'important');
                    });
                  }
                  // The sanitiser drops <body>, so the core hoists its colors
                  // into metas. They go back on every mail, dark rendering or
                  // not, and as the pair they are: white text written for a
                  // black <body> is white on the canvas without it, and a
                  // background alone, or the page left transparent with the
                  // default black text on it, is dark on dark. Put back as
                  // the inline declarations they were, with the priority they
                  // were written at; a surviving declaration of the mail's
                  // own is left alone.
                  function restoreDeclaredCanvas() {
                    var important = bodyMeta('meron-body-important').split(/\s+/);
                    [['background-color', 'meron-body-bg'], ['color', 'meron-body-fg']].forEach(function (pair) {
                      var value = bodyMeta(pair[1]);
                      if (!value || document.body.style.getPropertyValue(pair[0])) return;
                      document.body.style.setProperty(pair[0], value, important.indexOf(pair[0]) === -1 ? '' : 'important');
                    });
                  }
                  function applyDarkBody() {
                    var root = document.documentElement;
                    if (!root.classList.contains('meron-dark')) return;
                    var canvas = canvasTone();
                    if (canvas === 'dark') {
                      root.classList.remove('meron-dark');
                      return;
                    }
                    if (!canvas && textTone() === 'light') {
                      root.classList.remove('meron-dark');
                      return;
                    }
                    markBackgroundPictures();
                    capBackgrounds();
                  }
""".trim('\n')

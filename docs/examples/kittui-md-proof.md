# kittui-md proof gallery

## Components

A paragraph textbox with a [highlighted link](https://example.com), inline `code`, *emphasis*, **strong text**, ~~strikethrough~~, and inline math $x + y$.

![kittui placeholder image](assets/kittui-placeholder.png)

> A banner-style blockquote/callout rendered through the component layer.

## Lists

- bullet item
- [ ] unchecked task
- [x] checked task

3. ordered item starting at three
4. next ordered item

## Code and definitions

```rust
fn main() {
    println!("hello from kittui-md");
}
```

Term
: Definition text rendered as a definition-list component.

## Table

| Component | Status | Notes |
|:---|:---:|---:|
| textbox | implemented | wraps |
| link chip | implemented | highlighted |
| glyph table layout | modelled | relative anchors |

## Math and HTML

$$
a^2 + b^2 = c^2
$$

Inline HTML placeholder: <kbd>Ctrl</kbd> + <kbd>K</kbd>.

<div>Block HTML placeholder</div>

## Footnotes

A sentence with a footnote reference.[^proof]

[^proof]: Footnote definition rendered and exposed in metadata.

---

Footer text for the proof gallery.

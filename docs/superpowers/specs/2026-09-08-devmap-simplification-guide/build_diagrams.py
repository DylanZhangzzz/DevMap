"""Build editable draw.io pages and matching SVGs from diagrams.json."""
from pathlib import Path
import json
import html
import xml.etree.ElementTree as ET
from PIL import ImageFont

ROOT = Path(__file__).resolve().parent
FONT = Path('C:/Windows/Fonts/msyh.ttc')
COLORS = {
    'normal': ('#ffffff', '#cbd5e1'),
    'store': ('#e7f3ee', '#30916c'),
    'accent': ('#eaf2ff', '#6488c1'),
    'warning': ('#fff4e4', '#c49245'),
    'client': ('#eff0fc', '#9295c8'),
    'note': ('#edf1f6', '#cbd5e1'),
}


def wrap(value, width, size=17):
    font = ImageFont.truetype(str(FONT), size)
    lines, line = [], ''
    for char in value:
        if line and font.getlength(line + char) > width:
            lines.append(line)
            line = char
        else:
            line += char
    return lines + [line]


def sub(container, tag, **attrs):
    return ET.SubElement(container, tag, {k: str(v) for k, v in attrs.items()})


def svg_text(x, y, text, size=17, color='#40526a', weight='400'):
    return (f'<text x="{x}" y="{y}" font-size="{size}" fill="{color}" '
            f'font-weight="{weight}">{html.escape(text)}</text>')


def main():
    model = json.loads((ROOT / 'diagrams.json').read_text(encoding='utf-8'))
    mx = ET.Element('mxfile', host='app.diagrams.net', version='24.7.17', type='device')
    report = []
    for index, page in enumerate(model['pages'], 1):
        width, height = page['width'], page['height']
        diagram = sub(mx, 'diagram', id=page['id'], name=f'{index:02} {page["title"]}')
        gm = sub(diagram, 'mxGraphModel', dx=width, dy=height, grid=1, gridSize=10,
                 guides=1, tooltips=1, connect=1, arrows=1, fold=1, page=1,
                 pageScale=1, pageWidth=width, pageHeight=height, background='#f8fafc')
        root = sub(gm, 'root')
        sub(root, 'mxCell', id='0')
        sub(root, 'mxCell', id='1', parent='0')
        svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-labelledby="title desc">',
               f'<title id="title">{html.escape(page["title"])}</title>',
               f'<desc id="desc">{html.escape(page["subtitle"])}</desc>',
               '<defs><marker id="arrow" markerWidth="10" markerHeight="10" refX="8" refY="4" orient="auto" markerUnits="strokeWidth"><path d="M0,0 L8,4 L0,8 Z" fill="#64748b"/></marker></defs>',
               '<g font-family="Microsoft YaHei, Noto Sans CJK SC, sans-serif">',
               f'<rect width="{width}" height="{height}" fill="#f8fafc"/>',
               '<rect x="50" y="45" width="52" height="5" rx="2" fill="#30916c"/>',
               svg_text(50, 102, page['title'], 38, '#172c43', '700'),
               svg_text(50, 140, page['subtitle'], 17, '#586a81')]

        for tag, value, y, size in [('title', page['title'], 60, 38), ('subtitle', page['subtitle'], 117, 17)]:
            cell = sub(root, 'mxCell', id=tag, value=value, vertex=1, parent='1',
                       style=f'text;html=0;strokeColor=none;fillColor=none;align=left;verticalAlign=middle;fontFamily=Microsoft YaHei;fontSize={size};fontColor=#172c43;')
            geom = sub(cell, 'mxGeometry', x=50, y=y, width=1340, height=50 if tag == 'title' else 30)
            geom.set('as', 'geometry')

        by_id = {node['id']: node for node in page['nodes']}
        assert len(by_id) == len(page['nodes']), 'Duplicate diagram node ID'
        for edge_index, edge in enumerate(page['edges']):
            source, target = by_id[edge['from']], by_id[edge['to']]
            points = edge['points']
            path = 'M' + ' L'.join(f'{x},{y}' for x, y in points)
            svg.append(f'<path d="{path}" fill="none" stroke="#64748b" stroke-width="2" marker-end="url(#arrow)"/>')
            start, end = points[0], points[-1]
            ex = (start[0]-source['x'])/source['w']
            ey = (start[1]-source['y'])/source['h']
            tx = (end[0]-target['x'])/target['w']
            ty = (end[1]-target['y'])/target['h']
            assert all(0 <= v <= 1 for v in (ex, ey, tx, ty))
            style = (f'edgeStyle=orthogonalEdgeStyle;rounded=0;html=1;endArrow=block;endFill=1;strokeColor=#64748b;strokeWidth=2;'
                     f'fontSize=14;fontFamily=Microsoft YaHei;labelBackgroundColor=#f8fafc;exitX={ex};exitY={ey};exitDx=0;exitDy=0;entryX={tx};entryY={ty};entryDx=0;entryDy=0;')
            cell = sub(root, 'mxCell', id=f'edge-{edge_index}', value=edge['label'], style=style,
                       edge=1, parent='1', source=edge['from'], target=edge['to'])
            geom = sub(cell, 'mxGeometry', relative=1)
            geom.set('as', 'geometry')
            if len(points) > 2:
                arr = sub(geom, 'Array')
                arr.set('as', 'points')
                for x, y in points[1:-1]:
                    sub(arr, 'mxPoint', x=x, y=y)
            if edge['label']:
                a, b = max(zip(points, points[1:]), key=lambda pair: abs(pair[0][0]-pair[1][0]) + abs(pair[0][1]-pair[1][1]))
                x, y = (a[0]+b[0])/2, (a[1]+b[1])/2
                if a[0] == b[0]:
                    x += 12
                else:
                    x -= ImageFont.truetype(str(FONT), 14).getlength(edge['label'])/2
                    y -= 10
                svg.append(svg_text(x, y, edge['label'], 14))

        for node in page['nodes']:
            x, y, w, h = [node[k] for k in ('x', 'y', 'w', 'h')]
            assert x >= 0 and y >= 0 and x+w <= width and y+h <= height
            fill, stroke = COLORS[node['kind']]
            svg.append(f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="12" fill="{fill}" stroke="{stroke}" stroke-width="1.5"/>')
            title = html.escape(node['title'])
            if h < 85:
                svg.append(svg_text(x+18, y+38, node['title'], 20, '#172c43', '700'))
                body = ' '.join(node['lines'])
                svg.append(svg_text(x+165, y+38, body, 17))
                label = f'<span style="font-size:20px;font-weight:700">{title}</span>&nbsp;&nbsp;&nbsp;{html.escape(body)}'
                assert ImageFont.truetype(str(FONT), 17).getlength(body) <= w-183
            else:
                svg.append(svg_text(x+18, y+35, node['title'], 21, '#172c43', '700'))
                lines = [part for line in node['lines'] for part in wrap(line, w-36)]
                assert 61 + (len(lines)-1)*25 <= h-12, f'Overflow: {page["id"]}/{node["id"]}'
                for i, line in enumerate(lines):
                    svg.append(svg_text(x+18, y+61+i*25, line))
                label = f'<div style="font-size:21px;font-weight:700;line-height:25px;color:#172c43">{title}</div><div style="margin-top:2px;line-height:24px;font-size:17px;color:#40526a">' + '<br>'.join(html.escape(line) for line in lines) + '</div>'
            cell = sub(root, 'mxCell', id=node['id'], value=label, vertex=1, parent='1',
                       style=f'rounded=1;whiteSpace=wrap;html=1;fillColor={fill};strokeColor={stroke};strokeWidth=1.5;align=left;verticalAlign=top;spacing=12;fontFamily=Microsoft YaHei;fontSize=17;arcSize=10;')
            geom = sub(cell, 'mxGeometry', x=x, y=y, width=w, height=h)
            geom.set('as', 'geometry')
        svg += ['</g>', '</svg>']
        (ROOT / f'{page["file"]}.svg').write_text('\n'.join(svg), encoding='utf-8')
        report.append({'page': page['id'], 'nodes': len(page['nodes']), 'edges': len(page['edges']), 'bounds_and_text': 'pass'})

    ET.indent(mx)
    ET.ElementTree(mx).write(ROOT / 'devmap-simplification.drawio', encoding='utf-8', xml_declaration=True)
    # Parse generated formats independently; verify graph references and counts.
    parsed = ET.parse(ROOT / 'devmap-simplification.drawio')
    assert len(parsed.findall('diagram')) == len(model['pages'])
    for page in model['pages']:
        ET.parse(ROOT / f'{page["file"]}.svg')
    (ROOT / 'diagram-validation.json').write_text(json.dumps(report, indent=2, ensure_ascii=False), encoding='utf-8')
    print(json.dumps(report, ensure_ascii=False))


if __name__ == '__main__':
    main()

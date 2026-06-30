// 解析 JD 商品链接 → { youpin, inspect }。
//
// 典型链接:
//   https://item.m.jd.com/product/100213357832.html?inspectSkuId=124007259072525
//   youpinSkuId = 路径 /product/<youpin>.html 里的 <youpin>
//   inspectSkuId = query 里的 inspectSkuId(可能没有 → 留空让用户手动补)
//
// 容错:也接受裸数字(纯 youpinSkuId)、缺协议的链接、带其它 query 的链接。

export interface ParsedItem {
  youpin: string;
  inspect: string;
}

/** 从一段文本(整条 URL / 裸 id)解析出 youpin / inspect。解析不到的字段返回空串。 */
export function parseItemUrl(input: string): ParsedItem {
  const text = input.trim();
  if (!text) return { youpin: '', inspect: '' };

  // 纯数字 → 当作 youpinSkuId(用户只粘了商品 id)。
  if (/^\d+$/.test(text)) return { youpin: text, inspect: '' };

  let youpin = '';
  let inspect = '';

  // 路径里的 /product/<digits>.html → youpin。不依赖 URL 解析(缺协议也能匹配)。
  const m = text.match(/\/product\/(\d+)\.html/i);
  if (m) youpin = m[1];

  // query 里的 inspectSkuId=<digits>(大小写不敏感地找参数名)。
  const q = text.match(/[?&]inspectSkuId=(\d+)/i);
  if (q) inspect = q[1];

  // 兜底:若上面没匹配到 youpin,但文本里出现 /<digits>.html(无 /product/ 前缀)。
  if (!youpin) {
    const m2 = text.match(/\/(\d+)\.html/i);
    if (m2) youpin = m2[1];
  }

  return { youpin, inspect };
}

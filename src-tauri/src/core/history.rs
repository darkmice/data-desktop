//! 客户端本地持久化(SQLite,单文件 `app.db`)。两块职责:
//!
//! 1. **下单记录**(`orders` 表):每次提交(成功/失败)落一条,供「下单记录」
//!    视图查询/筛选/分页/统计/清空。纯本地、不分 token —— 单用户客户端的历史
//!    档案,与服务端的全局 SKU 库无关。一次提交一条(轮替取最终结果);全部
//!    保留,手动清空;`created_at` 毫秒,列表按时间倒序。
//!
//! 2. **设置快照**(`settings` 表,KV):config / 凭证 / 规则 的 JSON 快照,
//!    每次变更即写,启动时加载 —— 重启不丢、不用重新输入。单用户场景 KV 足够,
//!    无需为每类建表。
//!
//! 见 spec §4/§5。

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

/// 一条下单记录。字段与 `orders` 表一一对应;序列化用 camelCase 直接喂前端。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OrderRecord {
    /// 自增主键;新建记录时为 0,`insert` 后由 DB 赋值。
    #[serde(default)]
    pub id: i64,
    /// 下单时间(unix 毫秒)。
    pub created_at: i64,
    /// "success" | "failed"。
    pub status: String,
    /// "manual" | "auto"。
    pub trigger: String,
    pub inspect_sku_id: String,
    pub youpin_sku_id: String,
    pub short_name: String,
    pub price: String,
    pub quality: String,
    /// JD 相对图片路径(展示层拼 CDN 前缀);可能为空。
    #[serde(default)]
    pub image: String,
    /// 订单号(成功才有)。
    pub order_id: String,
    /// 使用的凭证名。
    pub credential: String,
    /// 失败原因原文(失败才有)。
    pub error: String,
    /// 自动触发时命中的规则 id(手动为空)。
    pub rule_id: String,
}

impl OrderRecord {
    /// 便捷构造:从下单结果的常见字段拼一条(id/created_at 由调用方或 insert 填)。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        created_at: i64,
        success: bool,
        trigger: &str,
        inspect_sku_id: &str,
        youpin_sku_id: &str,
        short_name: &str,
        price: &str,
        quality: &str,
        image: &str,
        order_id: &str,
        credential: &str,
        error: &str,
        rule_id: &str,
    ) -> Self {
        Self {
            id: 0,
            created_at,
            status: if success { "success" } else { "failed" }.into(),
            trigger: trigger.into(),
            inspect_sku_id: inspect_sku_id.into(),
            youpin_sku_id: youpin_sku_id.into(),
            short_name: short_name.into(),
            price: price.into(),
            quality: quality.into(),
            image: image.into(),
            order_id: order_id.into(),
            credential: credential.into(),
            error: error.into(),
            rule_id: rule_id.into(),
        }
    }
}

/// 统计汇总,给记录页顶部三个卡。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrderStats {
    pub total: i64,
    pub success: i64,
    pub failed: i64,
}

/// 列表过滤维度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Success,
    Failed,
}

impl Filter {
    pub fn from_str(s: &str) -> Self {
        match s {
            "success" => Filter::Success,
            "failed" => Filter::Failed,
            _ => Filter::All,
        }
    }
    /// 对应的 SQL WHERE 片段(含前导空格;All 为空)。
    fn where_clause(self) -> &'static str {
        match self {
            Filter::All => "",
            Filter::Success => " WHERE status = 'success'",
            Filter::Failed => " WHERE status = 'failed'",
        }
    }
}

/// 一页查询结果。
#[derive(Debug, Clone, Serialize)]
pub struct OrderPage {
    pub items: Vec<OrderRecord>,
    pub total: i64,
}

/// 本地下单记录库。单连接 + Mutex 串行化(写入频率极低,无需连接池)。
pub struct HistoryStore {
    conn: Mutex<Connection>,
}

impl HistoryStore {
    /// 打开(或新建)记录库并建表/建索引。
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// 内存库(单测用)。
    #[cfg(test)]
    pub fn open_in_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn init(conn: &Connection) -> anyhow::Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS orders (
                id             INTEGER PRIMARY KEY AUTOINCREMENT,
                created_at     INTEGER NOT NULL,
                status         TEXT    NOT NULL,
                trigger        TEXT    NOT NULL,
                inspect_sku_id TEXT    NOT NULL DEFAULT '',
                youpin_sku_id  TEXT    NOT NULL DEFAULT '',
                short_name     TEXT    NOT NULL DEFAULT '',
                price          TEXT    NOT NULL DEFAULT '',
                quality        TEXT    NOT NULL DEFAULT '',
                image          TEXT    NOT NULL DEFAULT '',
                order_id       TEXT    NOT NULL DEFAULT '',
                credential     TEXT    NOT NULL DEFAULT '',
                error          TEXT    NOT NULL DEFAULT '',
                rule_id        TEXT    NOT NULL DEFAULT ''
            );
            CREATE INDEX IF NOT EXISTS idx_orders_created ON orders(created_at DESC);

            CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        // 旧库迁移:补 image 列(已存在则忽略错误)。
        let _ = conn.execute("ALTER TABLE orders ADD COLUMN image TEXT NOT NULL DEFAULT ''", []);
        Ok(())
    }

    /// 读一个设置项的原始 JSON 字符串(无则 None)。
    pub fn kv_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let v = conn
            .query_row("SELECT value FROM settings WHERE key = ?1", params![key], |r| r.get::<_, String>(0))
            .ok();
        Ok(v)
    }

    /// 写一个设置项(JSON 字符串),覆盖。
    pub fn kv_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    /// 写一条,返回带 id 的记录(供 emit 给前端)。
    pub fn insert(&self, rec: &OrderRecord) -> anyhow::Result<OrderRecord> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orders
                (created_at, status, trigger, inspect_sku_id, youpin_sku_id,
                 short_name, price, quality, image, order_id, credential, error, rule_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                rec.created_at, rec.status, rec.trigger, rec.inspect_sku_id,
                rec.youpin_sku_id, rec.short_name, rec.price, rec.quality, rec.image,
                rec.order_id, rec.credential, rec.error, rec.rule_id,
            ],
        )?;
        let id = conn.last_insert_rowid();
        Ok(OrderRecord { id, ..rec.clone() })
    }

    /// 分页查询(按 created_at 倒序),返回 items + 过滤下总数。
    pub fn list(&self, filter: Filter, limit: i64, offset: i64) -> anyhow::Result<OrderPage> {
        let conn = self.conn.lock().unwrap();
        let where_clause = filter.where_clause();

        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM orders{where_clause}"),
            [],
            |r| r.get(0),
        )?;

        let sql = format!(
            "SELECT id, created_at, status, trigger, inspect_sku_id, youpin_sku_id,
                    short_name, price, quality, image, order_id, credential, error, rule_id
             FROM orders{where_clause}
             ORDER BY created_at DESC, id DESC
             LIMIT ?1 OFFSET ?2"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit, offset], Self::row_to_record)?;
        let items = rows.collect::<Result<Vec<_>, _>>()?;
        Ok(OrderPage { items, total })
    }

    /// 汇总统计。
    pub fn stats(&self) -> anyhow::Result<OrderStats> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM orders", [], |r| r.get(0))?;
        let success: i64 = conn.query_row(
            "SELECT COUNT(*) FROM orders WHERE status = 'success'",
            [],
            |r| r.get(0),
        )?;
        Ok(OrderStats { total, success, failed: total - success })
    }

    /// 清空所有记录。
    pub fn clear(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM orders", [])?;
        Ok(())
    }

    fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<OrderRecord> {
        Ok(OrderRecord {
            id: row.get(0)?,
            created_at: row.get(1)?,
            status: row.get(2)?,
            trigger: row.get(3)?,
            inspect_sku_id: row.get(4)?,
            youpin_sku_id: row.get(5)?,
            short_name: row.get(6)?,
            price: row.get(7)?,
            quality: row.get(8)?,
            image: row.get(9)?,
            order_id: row.get(10)?,
            credential: row.get(11)?,
            error: row.get(12)?,
            rule_id: row.get(13)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(t: i64, success: bool, trigger: &str, name: &str) -> OrderRecord {
        OrderRecord::new(
            t, success, trigger, "ins1", "yp1", name, "100.00", "99新", "jfs/x.jpg",
            if success { "ORD1" } else { "" },
            "测试凭证",
            if success { "" } else { "601" },
            if trigger == "auto" { "r1" } else { "" },
        )
    }

    #[test]
    fn insert_then_list_returns_with_id_desc() {
        let s = HistoryStore::open_in_memory().unwrap();
        let a = s.insert(&rec(1000, true, "manual", "A")).unwrap();
        let b = s.insert(&rec(2000, false, "auto", "B")).unwrap();
        assert!(a.id > 0 && b.id > a.id);

        let page = s.list(Filter::All, 10, 0).unwrap();
        assert_eq!(page.total, 2);
        // 倒序:created_at 2000 在前。
        assert_eq!(page.items[0].short_name, "B");
        assert_eq!(page.items[1].short_name, "A");
    }

    #[test]
    fn filter_success_and_failed() {
        let s = HistoryStore::open_in_memory().unwrap();
        s.insert(&rec(1, true, "manual", "ok1")).unwrap();
        s.insert(&rec(2, false, "auto", "bad1")).unwrap();
        s.insert(&rec(3, true, "manual", "ok2")).unwrap();

        assert_eq!(s.list(Filter::Success, 10, 0).unwrap().total, 2);
        assert_eq!(s.list(Filter::Failed, 10, 0).unwrap().total, 1);
        assert_eq!(s.list(Filter::Failed, 10, 0).unwrap().items[0].short_name, "bad1");
    }

    #[test]
    fn pagination_limit_offset() {
        let s = HistoryStore::open_in_memory().unwrap();
        for i in 0..5 {
            s.insert(&rec(i, true, "manual", &format!("n{i}"))).unwrap();
        }
        let p0 = s.list(Filter::All, 2, 0).unwrap();
        let p1 = s.list(Filter::All, 2, 2).unwrap();
        assert_eq!(p0.total, 5);
        assert_eq!(p0.items.len(), 2);
        assert_eq!(p1.items.len(), 2);
        // 倒序:n4,n3 | n2,n1
        assert_eq!(p0.items[0].short_name, "n4");
        assert_eq!(p1.items[0].short_name, "n2");
    }

    #[test]
    fn kv_set_get_overwrite() {
        let s = HistoryStore::open_in_memory().unwrap();
        assert!(s.kv_get("config").unwrap().is_none());
        s.kv_set("config", r#"{"token":"a"}"#).unwrap();
        assert_eq!(s.kv_get("config").unwrap().unwrap(), r#"{"token":"a"}"#);
        s.kv_set("config", r#"{"token":"b"}"#).unwrap();
        assert_eq!(s.kv_get("config").unwrap().unwrap(), r#"{"token":"b"}"#);
        // clear() 只清 orders,不动 settings。
        s.clear().unwrap();
        assert_eq!(s.kv_get("config").unwrap().unwrap(), r#"{"token":"b"}"#);
    }

    #[test]
    fn stats_and_clear() {
        let s = HistoryStore::open_in_memory().unwrap();
        s.insert(&rec(1, true, "manual", "a")).unwrap();
        s.insert(&rec(2, false, "auto", "b")).unwrap();
        s.insert(&rec(3, true, "manual", "c")).unwrap();
        let st = s.stats().unwrap();
        assert_eq!((st.total, st.success, st.failed), (3, 2, 1));

        s.clear().unwrap();
        assert_eq!(s.stats().unwrap().total, 0);
        assert_eq!(s.list(Filter::All, 10, 0).unwrap().total, 0);
    }
}

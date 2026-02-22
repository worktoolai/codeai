use anyhow::{Context, Result};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{BooleanQuery, Occur, QueryParser};
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy, TantivyDocument};

// ── Data types ──

#[derive(Debug, Clone)]
pub struct SearchDoc {
    pub symbol_id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub signature: String,
    pub doc: String,
    pub preview: String,
    pub strings: String,
}

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub symbol_id: String,
    pub name: String,
    pub path: String,
    pub kind: String,
    pub score: f32,
    pub matched_fields: Vec<String>,
    pub preview: String,
}

// ── SearchIndex ──

pub struct SearchIndex {
    index: Index,
    reader: IndexReader,
    schema: Schema,
    f_symbol_id: Field,
    f_name: Field,
    f_path: Field,
    f_kind: Field,
    f_signature: Field,
    f_doc: Field,
    f_preview: Field,
    f_strings: Field,
}

impl SearchIndex {
    pub fn open(index_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(index_dir)?;

        let mut schema_builder = Schema::builder();

        let f_symbol_id = schema_builder.add_text_field("symbol_id", STRING | STORED);
        let f_name = schema_builder.add_text_field("name", TEXT | STORED);
        let f_path = schema_builder.add_text_field("path", TEXT | STORED);
        let f_kind = schema_builder.add_text_field("kind", STRING | STORED);
        let f_signature = schema_builder.add_text_field("signature", TEXT | STORED);
        let f_doc = schema_builder.add_text_field("doc", TEXT | STORED);
        let f_preview = schema_builder.add_text_field("preview", TEXT | STORED);
        let f_strings = schema_builder.add_text_field("strings", TEXT | STORED);

        let schema = schema_builder.build();

        let index = if index_dir.join("meta.json").exists() {
            Index::open_in_dir(index_dir).context("open tantivy index")?
        } else {
            Index::create_in_dir(index_dir, schema.clone()).context("create tantivy index")?
        };

        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .context("create index reader")?;

        Ok(Self {
            index,
            reader,
            schema,
            f_symbol_id,
            f_name,
            f_path,
            f_kind,
            f_signature,
            f_doc,
            f_preview,
            f_strings,
        })
    }

    pub fn writer(&self) -> Result<IndexWriter> {
        self.index
            .writer(50_000_000) // 50MB heap
            .context("create index writer")
    }

    pub fn index_block(&self, writer: &IndexWriter, block: &SearchDoc) -> Result<()> {
        // Delete existing doc with same symbol_id
        let term = tantivy::Term::from_field_text(self.f_symbol_id, &block.symbol_id);
        writer.delete_term(term);

        writer.add_document(doc!(
            self.f_symbol_id => block.symbol_id.clone(),
            self.f_name => block.name.clone(),
            self.f_path => block.path.clone(),
            self.f_kind => block.kind.clone(),
            self.f_signature => block.signature.clone(),
            self.f_doc => block.doc.clone(),
            self.f_preview => block.preview.clone(),
            self.f_strings => block.strings.clone(),
        ))?;

        Ok(())
    }

    pub fn delete_by_path(&self, writer: &IndexWriter, path: &str) -> Result<()> {
        let term = tantivy::Term::from_field_text(self.f_path, path);
        writer.delete_term(term);
        Ok(())
    }

    pub fn search(
        &self,
        query_str: &str,
        limit: usize,
        path_filter: Option<&str>,
        _lang_filter: Option<&str>,
    ) -> Result<Vec<SearchHit>> {
        let searcher = self.reader.searcher();

        // Build query with boosted fields
        let mut query_parser = QueryParser::for_index(
            &self.index,
            vec![
                self.f_name,
                self.f_signature,
                self.f_doc,
                self.f_preview,
                self.f_strings,
                self.f_path,
            ],
        );
        query_parser.set_conjunction_by_default();

        let base_query = query_parser.parse_query(query_str).context("parse query")?;

        // Apply path filter if provided
        let final_query: Box<dyn tantivy::query::Query> = if let Some(pf) = path_filter {
            let path_term = tantivy::Term::from_field_text(self.f_path, pf);
            let path_query = tantivy::query::TermQuery::new(path_term, IndexRecordOption::Basic);
            Box::new(BooleanQuery::new(vec![
                (Occur::Must, base_query),
                (Occur::Must, Box::new(path_query)),
            ]))
        } else {
            base_query
        };

        let top_docs = searcher
            .search(&final_query, &TopDocs::with_limit(limit))
            .context("search")?;

        let mut hits = Vec::new();
        for (score, doc_address) in top_docs {
            let doc: TantivyDocument = searcher.doc(doc_address).context("retrieve doc")?;

            let symbol_id = get_text(&doc, self.f_symbol_id);
            let name = get_text(&doc, self.f_name);
            let path = get_text(&doc, self.f_path);
            let kind = get_text(&doc, self.f_kind);
            let preview = get_text(&doc, self.f_preview);

            // Determine which fields matched
            let mut matched_fields = Vec::new();
            let q_lower = query_str.to_lowercase();
            if name.to_lowercase().contains(&q_lower) {
                matched_fields.push("name".to_string());
            }
            if get_text(&doc, self.f_doc).to_lowercase().contains(&q_lower) {
                matched_fields.push("doc".to_string());
            }
            if get_text(&doc, self.f_strings).to_lowercase().contains(&q_lower) {
                matched_fields.push("str".to_string());
            }
            if path.to_lowercase().contains(&q_lower) {
                matched_fields.push("path".to_string());
            }
            if get_text(&doc, self.f_signature).to_lowercase().contains(&q_lower) {
                matched_fields.push("sig".to_string());
            }
            if matched_fields.is_empty() {
                matched_fields.push("preview".to_string());
            }

            hits.push(SearchHit {
                symbol_id,
                name,
                path,
                kind,
                score,
                matched_fields,
                preview,
            });
        }

        Ok(hits)
    }

    pub fn reload(&self) -> Result<()> {
        self.reader.reload().context("reload reader")
    }

    pub fn clear_all(&self) -> Result<()> {
        let mut writer = self.writer()?;
        writer.delete_all_documents()?;
        writer.commit()?;
        Ok(())
    }
}

fn get_text(doc: &TantivyDocument, field: Field) -> String {
    doc.get_first(field)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

use diesel::prelude::*;
use papergraph;
use papergraph::db::models;
use papergraph::io::Paper;
use serde_json;
use std::fs::File;
use std::io::{self, BufRead};

use clap::Clap;

/// Inser records into the database
#[derive(Clap, Debug)]
#[clap(version = "0.1.0", author = "Denny Britz")]
struct Opts {
    /// Read JSON records from this path
    #[clap(short = "d", long = "data")]
    data: String,

    /// Ignore papers with fewer incoming citations than this
    #[clap(
        long = "min-citations",
        short = "mc",
        default_value = "1",
        required = false
    )]
    min_citations: usize,

    /// Only insert papers with these fields of study
    #[clap(
        long = "field_of_study",
        short = "fos",
        default_value = "Computer Science",
        multiple = true,
        required = false
    )]
    field_of_study: Vec<String>,
}

fn main() {
    env_logger::init();
    let opts: Opts = Opts::parse();

    log::info!("establishing db connection");
    let conn = papergraph::db::establish_connection();

    log::info!("reading records from {}", &opts.data);
    let file = File::open(&opts.data).expect("failed to open data file");

    let min_citations = opts.min_citations;
    let records = io::BufReader::new(file)
        .lines()
        .map(|l| l.expect("failed to read line"))
        .map(|l| serde_json::from_str(&l).expect("failed to parse paper"))
        .filter(|p: &Paper| p.in_citations.len() > min_citations)
        .filter(|p: &Paper| {
            opts.field_of_study
                .iter()
                .any(|fos| p.fields_of_study.contains(fos))
        });

    // TODO: batch this iterator
    records.for_each(|record| {
        // Insert paper
        diesel::insert_into(papergraph::db::schema::papers::table)
            .values(&models::Paper {
                id: record.id.clone(),
                title: record.title.clone(),
                year: record.year.map(|y| y as i16),
            })
            .on_conflict(papergraph::db::schema::papers::id)
            // TODO: Should upsert here
            .do_nothing()
            .execute(&conn)
            .expect("error storing paper");

        // Insert authors
        let authors: Vec<models::Author> = record
            .authors
            .iter()
            // TODO: Is it correct to filter out authors without ID!?
            .filter(|a| a.ids.len() > 0)
            .map(|a| models::Author {
                id: a.ids.get(0).unwrap().clone(),
                name: a.name.clone(),
            })
            .collect();

        diesel::insert_into(papergraph::db::schema::authors::table)
            .values(&authors)
            .on_conflict(papergraph::db::schema::authors::id)
            .do_nothing()
            .execute(&conn)
            .expect("error storing authors");

        // Insert author relationships
        let paper_authors: Vec<models::PaperAuthor> = authors.iter().map(|a|
            models::PaperAuthor {
                paper_id: &record.id,
                author_id: &a.id,
            }
        ).collect();

        diesel::insert_into(papergraph::db::schema::paper_authors::table)
            .values(&paper_authors)
            .on_conflict((papergraph::db::schema::paper_authors::paper_id, papergraph::db::schema::paper_authors::author_id))
            .do_nothing()
            .execute(&conn)
            .expect("error storing paper_authors");

        // Insert citations
        // TODO: What if we have more than 64k citations - need to batch?
        let mut citations: Vec<models::Citation> = vec![];
        citations.extend(
            record
                .out_citations
                .iter()
                .map(|to_paper| models::Citation {
                    from_paper: &record.id,
                    to_paper: to_paper,
                }),
        );
        citations.extend(
            record
                .in_citations
                .iter()
                .map(|from_paper| models::Citation {
                    from_paper: from_paper,
                    to_paper: &record.id,
                }),
        );

        diesel::insert_into(papergraph::db::schema::citations::table)
            .values(&citations)
            .on_conflict((
                papergraph::db::schema::citations::from_paper,
                papergraph::db::schema::citations::to_paper,
            ))
            .do_nothing()
            .execute(&conn)
            .expect("error storing citatins");
        log::debug!("inserted {} [{} citations]", &record.id, citations.len());
    });
}

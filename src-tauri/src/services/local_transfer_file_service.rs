use sqlx::{Row, SqlitePool};

use crate::models::transfer::LocalTransferFile;

#[derive(Clone)]
pub struct LocalTransferFileService {
    pool: SqlitePool,
}

impl LocalTransferFileService {
    pub async fn new(pool: SqlitePool) -> Self {
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS local_transfer_files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                transfer_id TEXT NOT NULL,
                file_path TEXT NOT NULL,
                file_name TEXT NOT NULL,
                file_size INTEGER NOT NULL
            )
            "#,
        )
        .execute(&pool)
        .await
        .expect("Failed creating local_transfer_files table");

        Self { pool }
    }

    // pub async fn insert(
    //     &self,
    //     file: LocalTransferFile,
    // ) -> Result<(), sqlx::Error> {

    //     sqlx::query(
    //         r#"
    //         INSERT INTO local_transfer_files
    //         (
    //             transfer_id,
    //             file_path,
    //             file_name,
    //             file_size
    //         )
    //         VALUES (?, ?, ?, ?)
    //         "#
    //     )
    //     .bind(file.transfer_id)
    //     .bind(file.file_path)
    //     .bind(file.file_name)
    //     .bind(file.file_size as i64)
    //     .execute(&self.pool)
    //     .await?;

    //     Ok(())
    // }

    pub async fn insert(&self, file: LocalTransferFile) -> Result<(), sqlx::Error> {
        tracing::info!(
            transfer_id = %file.transfer_id,
            file_path = %file.file_path,
            file_name = %file.file_name,
            file_size = file.file_size,
            "Inserting local transfer file"
        );

        sqlx::query(
            r#"
        INSERT INTO local_transfer_files
        (
            transfer_id,
            file_path,
            file_name,
            file_size
        )
        VALUES (?, ?, ?, ?)
        "#,
        )
        .bind(&file.transfer_id)
        .bind(&file.file_path)
        .bind(&file.file_name)
        .bind(file.file_size as i64)
        .execute(&self.pool)
        .await?;

        tracing::info!(
            transfer_id = %file.transfer_id,
            "Local transfer file inserted"
        );

        Ok(())
    }

    // pub async fn get_by_transfer_id(
    //     &self,
    //     transfer_id: &str,
    // ) -> Result<Vec<LocalTransferFile>, sqlx::Error> {
    //     let rows = sqlx::query(
    //         r#"
    //         SELECT
    //             transfer_id,
    //             file_path,
    //             file_name,
    //             file_size
    //         FROM local_transfer_files
    //         WHERE transfer_id = ?
    //         "#,
    //     )
    //     .bind(transfer_id)
    //     .fetch_all(&self.pool)
    //     .await?;

    //     let files = rows
    //         .into_iter()
    //         .map(|row| LocalTransferFile {
    //             transfer_id: row.get("transfer_id"),
    //             file_path: row.get("file_path"),
    //             file_name: row.get("file_name"),
    //             file_size: row.get::<i64, _>("file_size") as u64,
    //         })
    //         .collect();

    //     Ok(files)
    // }

    pub async fn get_by_transfer_id(
        &self,
        transfer_id: &str,
    ) -> Result<Vec<LocalTransferFile>, sqlx::Error> {
        tracing::info!(
            transfer_id = %transfer_id,
            "Looking up local files"
        );

        let rows = sqlx::query(
            r#"
        SELECT
            transfer_id,
            file_path,
            file_name,
            file_size
        FROM local_transfer_files
        WHERE transfer_id = ?
        "#,
        )
        .bind(transfer_id)
        .fetch_all(&self.pool)
        .await?;

        tracing::info!(
            transfer_id = %transfer_id,
            count = rows.len(),
            "Lookup complete"
        );

        for row in &rows {
            tracing::info!(
                transfer_id = %row.get::<String,_>("transfer_id"),
                file_path = %row.get::<String,_>("file_path"),
                "Found file"
            );
        }

        let files = rows
            .into_iter()
            .map(|row| LocalTransferFile {
                transfer_id: row.get("transfer_id"),
                file_path: row.get("file_path"),
                file_name: row.get("file_name"),
                file_size: row.get::<i64, _>("file_size") as u64,
            })
            .collect();

        Ok(files)
    }

    pub async fn delete_by_transfer_id(&self, transfer_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            DELETE FROM local_transfer_files
            WHERE transfer_id = ?
            "#,
        )
        .bind(transfer_id)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn delete_file(&self, transfer_id: &str, file_path: &str) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            DELETE FROM local_transfer_files
            WHERE transfer_id = ?
            AND file_path = ?
            "#,
        )
        .bind(transfer_id)
        .bind(file_path)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn exists(&self, transfer_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            SELECT COUNT(*) as count
            FROM local_transfer_files
            WHERE transfer_id = ?
            "#,
        )
        .bind(transfer_id)
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = result.get("count");

        Ok(count > 0)
    }
}

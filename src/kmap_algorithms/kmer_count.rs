use std::collections::HashMap;
use std::path::Path;
use bio::io::fasta;

pub type KmerHash = u64;

#[allow(dead_code)]
/// Converts a hash value back to a k-mer.
/// 
/// # Arguments
/// 
/// * `kmer_hash` - A u64 value representing the hash of the k-mer to convert back.
/// * `kmer_length` - The length of the k-mer to generate.
/// 
/// # Returns
/// A string containing the k-mer represented by the input hash value.
pub fn hash2kmer(kmer_hash: KmerHash, kmer_length: usize) -> Vec<u8> {
    let mut kmer = Vec::with_capacity(kmer_length);
    let mut hash = kmer_hash;

    for _ in 0..kmer_length {
        let base = match hash & 3 {
            0 => 'A',
            1 => 'C',
            2 => 'G',
            _ => 'T',
        };
        kmer.push(base as u8);
        hash >>= 2;
    }

    kmer.into_iter().rev().collect()
}

/// Finds the first valid k-mer in a sequence starting from a given position.
/// 
/// # Arguments
/// 
/// * `sequence` - A string slice that holds the sequence to search for k-mers.
/// * `kmer_length` - The length of the k-mers to search for.
/// * `start_pos` - The starting position in the sequence to begin searching.
/// * `valid_bases` - A slice of characters that are considered valid for k-mers.
/// 
/// # Returns
/// 
/// An Option containing a tuple of the hash value of the first valid k-mer and its position in the sequence, or None if no valid k-mer is found.
pub fn find_first_valid_kmer(sequence: &[u8], kmer_length: usize, start_pos: usize, valid_bases: &[u8]) -> Option<(KmerHash, usize)> {
    let sequence_length = sequence.len();
    for i in start_pos..=sequence_length.saturating_sub(kmer_length) {
        let kmer = &sequence[i..i+kmer_length];
        if kmer.iter().all(|&base| valid_bases.contains(&base)) {
            let kmer_hash = kmer2hash(kmer);
            return Some((kmer_hash, i + kmer_length));
        }
    }
    None
}

/// Counts the k-mers in a given sequence.
/// 
/// # Arguments
/// 
/// * `sequence` - A byte slice that holds the sequence to analyze.
/// * `kmer_length` - The length of the k-mers to count.
/// * `revcom_mode` - A boolean indicating whether to count reverse complements (true) or not (false).
///
/// # Returns
///
/// A HashMap where the keys are k-mer hashes and the values are their counts.
pub fn count_kmers_in_one_sequence(sequence: &[u8], kmer_length: usize, revcom_mode: bool) -> HashMap<KmerHash, u32> {
    if !sequence.iter().all(|&b| b.is_ascii_uppercase()) {
        panic!("Input sequence must be all uppercase.");
    }
    let mut kmer_table = HashMap::new();
    let valid_bases = [b'A', b'T', b'C', b'G'];
    let base_to_num = |b: u8| match b {
        b'A' => 0,
        b'C' => 1,
        b'G' => 2,
        b'T' => 3,
        _ => unreachable!(),
    };

    let sequence_length = sequence.len();
    let hash_mask = (1 << (2 * kmer_length)) - 1;

    let i = 0;

    if let Some((mut kmer_hash, mut i)) = find_first_valid_kmer(sequence, kmer_length, i, &valid_bases) {
        *kmer_table.entry(kmer_hash).or_insert(0) += 1;

        while i < sequence_length {
            let base = sequence[i];
            if valid_bases.contains(&base) {
                kmer_hash = ((kmer_hash << 2) & hash_mask) | base_to_num(base) as KmerHash;
                *kmer_table.entry(kmer_hash).or_insert(0) += 1;
                i += 1;
            } else {
                if let Some((new_hash, new_i)) = find_first_valid_kmer(sequence, kmer_length, i + 1, &valid_bases) {
                    kmer_hash = new_hash;
                    i = new_i;
                    *kmer_table.entry(kmer_hash).or_insert(0) += 1;
                } else {
                    break;
                }
            }
        }
    }

    if revcom_mode {
        let mut rc_table = HashMap::new();
        for (&kmer_hash, &count) in kmer_table.iter() {
            let rc_hash = revcom_hash(kmer_hash, kmer_length);
            *rc_table.entry(rc_hash).or_insert(0) += count;
        }
        // Merge rc_table into kmer_table
        for (rc_hash, rc_count) in rc_table {
            *kmer_table.entry(rc_hash).or_insert(0) += rc_count;
        }
    }
    kmer_table
}

/// Counts the k-mers in a vector of sequences.
/// 
/// # Arguments
/// 
/// * `sequences` - A vector of Vec<u8>, where each inner Vec<u8> represents a sequence.
/// * `kmer_length` - The length of the k-mers to count.
/// * `revcom_mode` - A boolean indicating whether to count reverse complements (true) or not (false).
/// 
/// # Returns
/// 
/// A HashMap where the keys are k-mer hashes and the values are their counts.
pub fn count_kmers_in_sequences(sequences: &[Vec<u8>], kmer_length: usize, revcom_mode: bool) -> HashMap<KmerHash, u32> {
    if kmer_length > 31 {
        panic!("Kmer length > 31 is not supported");
    }
    let mut kmer_table = HashMap::new();

    for sequence in sequences {
        let sequence_kmer_table = count_kmers_in_one_sequence(sequence, kmer_length, revcom_mode);
        for (kmer_hash, count) in sequence_kmer_table {
            *kmer_table.entry(kmer_hash).or_insert(0) += count;
        }
    }

    kmer_table
}

/// Converts a kmer represented as a byte slice into its corresponding kmer hash.
/// 
/// # Arguments
/// 
/// * `kmer` - A slice of bytes representing the kmer (e.g., b"A", b"T", b"C", b"G").
/// 
/// # Returns
/// 
/// A `Result<KmerHash, String>` representing the hash value of the input kmer. If the input contains any invalid bases, an error message is returned.
pub fn kmer2hash(kmer: &[u8]) -> KmerHash {
    let mut hash_value: KmerHash = 0;

    for &base in kmer {
        hash_value <<= 2;
        match base {
            b'C' => hash_value |= 1,
            b'G' => hash_value |= 2,
            b'T' => hash_value |= 3,
            b'A' => {} // No action needed for 'A'
            _ => panic!("Invalid base: {}, should only contain A C G T.", base as char), // Panic for any invalid bases
        }
    }

    hash_value // Return the hash value
}

#[allow(dead_code)]
/// Returns the reverse complement of a given DNA sequence.
/// 
/// # Arguments
/// 
/// * `seq` - A slice of bytes representing the DNA sequence (e.g., b"A", b"T", b"C", b"G").
/// 
/// # Returns
/// 
/// A `Vec<u8>` containing the reverse complement of the input sequence.
pub fn reverse_complement(seq: &[u8]) -> Vec<u8> {
    seq.iter().rev().map(|&b| match b {
        b'A' | b'a' => b'T',
        b'T' | b't' => b'A',
        b'C' | b'c' => b'G',
        b'G' | b'g' => b'C',
        _ => b,
    }).collect()
}

/// Calculates the hash value of the reverse complement of a kmer.
///
/// # Arguments
///
/// * `kmer_hash` - The hash value of the original kmer.
/// * `kmer_length` - The length of the kmer.
///
/// # Returns
///
/// The hash value of the reverse complement of the kmer.
pub fn revcom_hash(kmer_hash: KmerHash, kmer_length: usize) -> KmerHash {
    let mut rc_hash: KmerHash = 0;
    let mut temp_hash = kmer_hash;
    
    for _ in 0..kmer_length {
        rc_hash <<= 2;
        rc_hash |= 3 - (temp_hash & 3);  // 3 - x gives the complement for 2-bit encoding
        temp_hash >>= 2;
    }
    
    rc_hash
}




/// Loads sequences from a FASTA file into a vector.
///
/// # Arguments
///
/// * `fasta_file_path` - A string slice that holds the path to the FASTA file.
///
/// # Returns
///
/// A Vec of Vec<u8>, where each inner Vec<u8> represents a sequence in uppercase.
///
/// # Panics
///
/// This function will panic if there's any error reading the FASTA file.
pub fn load_fasta(fasta_file_path: &str) -> Vec<Vec<u8>> {
    let path = Path::new(fasta_file_path);
    let reader = fasta::Reader::from_file(path)
        .unwrap_or_else(|_| panic!("Error in opening fasta file: {}", fasta_file_path));

    reader.records()
        .map(|record| record.unwrap().seq().to_ascii_uppercase().to_vec())
        .collect()
}

/// Removes empty sequences from a FASTA file and writes non-empty sequences to a new file
///
/// # Arguments
///
/// * `input_fa_file` - Path to the input FASTA file
/// * `output_fa_file` - Path to write the filtered FASTA file
///
/// # Returns
///
/// Result containing the number of sequences removed and total sequences processed
pub fn remove_empty_seq(
    input_fa_file: &str,
    output_fa_file: &str,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    let path = Path::new(input_fa_file);
    let reader = fasta::Reader::from_file(path)?;
    
    // Create output file
    let output_path = Path::new(output_fa_file);
    let mut writer = fasta::Writer::to_file(output_path)?;
    
    let mut empty_count = 0;
    let mut total_count = 0;
    
    // Process each record
    for result in reader.records() {
        total_count += 1;
        if let Ok(record) = result {
            let seq = record.seq();
            if !seq.is_empty() {
                // Write non-empty sequence to output file
                writer.write_record(&record)?;
            } else {
                empty_count += 1;
                println!("Removing empty sequence: {}", record.id());
            }
        }
    }
    
    println!("FASTA processing complete:");
    println!("Total sequences: {}", total_count);
    println!("Empty sequences removed: {}", empty_count);
    println!("Sequences written: {}", total_count - empty_count);
    println!("Output written to: {}", output_fa_file);
    
    Ok((empty_count, total_count))
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_kmer2hash() {
        let kmer1 = b"ATCG";
        let kmer2 = b"AAAAAAAAAAAAAAAAA"; // 17 A's
        let kmer4 = b"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"; // 31 A's
        let kmer6 = b"CATGC";

        let kmer_hash1 = kmer2hash(kmer1);
        let kmer_hash2 = kmer2hash(kmer2);
        let kmer_hash4 = kmer2hash(kmer4);
        let kmer_hash6 = kmer2hash(kmer6);

        assert_eq!(kmer_hash1, 54); // Expected hash for ATCG
        assert_eq!(kmer_hash2, 0); // Expected hash for 17 A's
        assert_eq!(kmer_hash4, 0); // Expected hash for 31 A's
        assert_eq!(kmer_hash6, 313); // Expected hash for CATGC
    }

    #[test]
    #[should_panic]
    fn test_kmer2hash_panic() {
        let kmer = b"ANGTC";
        kmer2hash(kmer);
    }

    
    #[test]
    fn test_hash2kmer() {
        let kmer_hash = 54;
        let kmer_length = 4; // Example length

        let result: Vec<u8> = hash2kmer(kmer_hash, kmer_length);
        
        // Example expected result as Vec<u8>
        let expected: Vec<u8> = b"ATCG".to_vec(); // Adjust this based on your expected output

        assert_eq!(result, expected); // Compare the result with the expected Vec<u8>
    }

    #[test]
    fn test_reverse_complement() {
        let seq1 = b"TGTACAAAGCCCTGCATGTT";
        let rev_com_seq1 = reverse_complement(seq1);
        assert_eq!(rev_com_seq1, b"AACATGCAGGGCTTTGTACA");

        let seq2 = b"TCAAG";
        let rev_com_seq2 = reverse_complement(seq2);
        assert_eq!(rev_com_seq2, b"CTTGA");

        let seq3 = b"ACGNT";
        let rev_com_seq3 = reverse_complement(seq3);
        assert_eq!(rev_com_seq3, b"ANCGT");
    }

    #[test]
    fn test_find_first_valid_kmer() {
        let sequence1 = b"ATGCATGCATGCATGCATGC";
        let kmer_length1 = 3;
        let valid_bases: &[u8] = b"ATGC";
        let result1 = find_first_valid_kmer(sequence1, kmer_length1, 0, valid_bases);
        assert_eq!(result1, Some((14, 3))); // ATG: 00 11 10

        let sequence2 = b"ATNATGCATGCATGCATGC";
        let kmer_length2 = 4;
        let result2 = find_first_valid_kmer(sequence2, kmer_length2, 0, valid_bases);
        assert_eq!(result2, Some((57, 7))); // ATGC: 00 11 10 01

        let sequence3 = b"ATGCNNATGNCATGCATGC";
        let kmer_length3 = 5;
        let result3 = find_first_valid_kmer(sequence3, kmer_length3, 3, valid_bases);
        assert_eq!(result3, Some((313, 15))); // CATGC: 01 00 11 10 01
    }

    #[test]
    fn test_count_kmers_in_one_sequence() {
        let sequence = b"ATGCAT";
        let kmer_length = 3;
        
        // Test without reverse complement mode
        let kmer_table = count_kmers_in_one_sequence(sequence, kmer_length, false);
        assert_eq!(kmer_table, HashMap::from([(14, 1), (57, 1), (36, 1), (19, 1)]));

        // Test with reverse complement mode
        let kmer_table_rc = count_kmers_in_one_sequence(sequence, kmer_length, false);
        let mut tmp_tbl = HashMap::new();
        for (hash, cnt) in count_kmers_in_one_sequence(sequence, kmer_length, false) {
            tmp_tbl.insert(hash, cnt);
        }
        for (hash, cnt) in count_kmers_in_one_sequence(&reverse_complement(sequence), kmer_length, false) {
            tmp_tbl.insert(hash, cnt);
        }
        assert_eq!(kmer_table_rc, tmp_tbl);

        // Other tests...
        let sequence1 = b"ATGNCAT";
        let kmer_table1 = count_kmers_in_one_sequence(sequence1, kmer_length, false);
        assert_eq!(kmer_table1, HashMap::from([(14, 1), (19, 1)]));

        let sequence2 = b"ATGNNNAT";
        let kmer_table2 = count_kmers_in_one_sequence(sequence2, kmer_length, false);
        assert_eq!(kmer_table2, HashMap::from([(14, 1)]));

        let sequence3 = b"ATGNNNAT";
        let kmer_length3 = 4;
        let kmer_table3 = count_kmers_in_one_sequence(sequence3, kmer_length3, false);
        assert_eq!(kmer_table3, HashMap::new());

        let sequence4 = b"ATGNNNATACNCCCA";
        let kmer_length4 = 4;
        let kmer_table4 = count_kmers_in_one_sequence(sequence4, kmer_length4, false);
        assert_eq!(kmer_table4, HashMap::from([(49, 1), (84, 1)]));

        let sequence5 = b"ATGNNNATACNCCCANCCCA";
        let kmer_length5 = 4;
        let kmer_table5 = count_kmers_in_one_sequence(sequence5, kmer_length5, false);
        assert_eq!(kmer_table5, HashMap::from([(49, 1), (84, 2)]));
    }

    #[test]
    fn test_count_kmers_in_sequences() {
        let fasta_file_path = "./tests/test2.fa"; // Make sure this path is correct
        let fasta_file_path_1 = "./tests/test3.fa"; 
        let kmer_length = 3;
        
        // Test with revcom_mode = true
        let sequences = load_fasta(fasta_file_path);
        let kmer_table_with_revcom = count_kmers_in_sequences(&sequences, kmer_length, true);
        assert_eq!(kmer_table_with_revcom, HashMap::from([(48, 2), (0, 6), (3, 2), (15, 2), (60, 2), (1, 1), (63, 6), (47, 1)]));

        // Test with revcom_mode = false
        let sequences_1 = load_fasta(fasta_file_path_1);
        let kmer_table_without_revcom = count_kmers_in_sequences(&sequences_1, kmer_length, false);
        assert_eq!(kmer_table_without_revcom, HashMap::from([(48,1),(0,3)]));

        // Test for kmer_length = 33
        let kmer_length_invalid = 33;
        let result = std::panic::catch_unwind(|| count_kmers_in_sequences(&sequences, kmer_length_invalid, true));
        assert!(result.is_err());
    }

    #[test]
    fn test_revcom_hash() {

        // Test case 2: AAAAAA (hash: 0) -> TTTTTT (hash: 4095)
        assert_eq!(revcom_hash(0, 6), 4095);

        // Test case 3: CATG (hash: 228) -> CATG (hash: 228)
        assert_eq!(revcom_hash(228, 4), 228);

        // Test case 4: A (hash: 0) -> T (hash: 3)
        assert_eq!(revcom_hash(0, 1), 3);

    }

    #[test]
    fn test_revcom_hash_with_reverse_complement() {
        let kmer = b"AACGT";
        let kmer_length = kmer.len();
        let kmer_hash = kmer2hash(kmer);

        // Calculate reverse complement hash using revcom_hash function
        let rc_hash_calculated = revcom_hash(kmer_hash, kmer_length);

        // Calculate reverse complement hash by actually reversing and complementing the sequence
        let rc_seq = reverse_complement(kmer);
        let rc_hash_actual = kmer2hash(&rc_seq);

        // Compare the results
        assert_eq!(rc_hash_calculated, rc_hash_actual, 
            "revcom_hash result doesn't match the hash of the actual reverse complement sequence");

        // Additional test cases
        let test_cases: &[&[u8]] = &[
            b"AAAAA",
            b"CCCCC",
            b"GGGGG",
            b"TTTTT",
            b"ACGTACGT",
            b"AATTCCGG",
            b"GCATGCAT",
        ];
        for &test_kmer in test_cases {
            let test_kmer_length = test_kmer.len();
            let test_kmer_hash = kmer2hash(test_kmer);
            let test_rc_hash_calculated = revcom_hash(test_kmer_hash, test_kmer_length);
            let test_rc_seq = reverse_complement(test_kmer);
            let test_rc_hash_actual = kmer2hash(&test_rc_seq);

            assert_eq!(test_rc_hash_calculated, test_rc_hash_actual, 
                "revcom_hash result doesn't match the hash of the actual reverse complement sequence for kmer {:?}", 
                std::str::from_utf8(test_kmer).unwrap());
        }
    }
    
    #[test]
    fn test_load_fasta() {
        let fasta_file_path = "./tests/test2.fa"; // Make sure this path is correct
        let sequences = load_fasta(fasta_file_path);
        
        assert_eq!(sequences.len(), 2); // Assuming test2.fa contains two sequences
        assert_eq!(sequences[0], b"TAAAAAATTA");
        assert_eq!(sequences[1], b"TNAAACNAAA");

        // Test with a non-existent file
        let non_existent_file = "./tests/non_existent.fa";
        let result = std::panic::catch_unwind(|| load_fasta(non_existent_file));
        assert!(result.is_err());
    }

}

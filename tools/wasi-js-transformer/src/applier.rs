// Module to update and insert bytes into the wasm binary

use crate::generator::*;
use crate::parser::*;
use crate::utils::*;
use std::*;

/// Function to add/edit bytes in the binary
pub fn apply_transformations_to_wasm_binary_vec(
    mut wasm_binary_vec: &mut Vec<u8>,
    imported_i64_functions: &[&WasmFunction],
    trampoline_functions: &[TrampolineFunction],
    lowered_signatures: &[LoweredSignature],
    wasm_sections: &[WasmSection],
    type_signatures: &[WasmTypeSignature],
    wasm_functions: &[WasmFunction],
    wasm_calls: &[WasmCall],
) -> Result<(), &'static str> {
    // Must apply updates in order acording to the binary spec to preserve the position offset,
    // https://github.com/WebAssembly/design/blob/master/BinaryEncoding.md#high-level-structure

    // Offset from the original position from our sections.
    // This should be updated as bytes are added.
    let mut position_offset: usize = 0;

    // Add the new lowered signatures to the Types Section
    let types_section = wasm_sections
        .iter()
        .find(|&x| x.code == WasmSectionCode::Type)
        .unwrap();
    let lowered_signature_bytes = lowered_signatures
        .iter()
        .cloned()
        .map(|ls| ls.bytes)
        .collect::<Vec<_>>();
    position_offset += add_entries_to_section(
        wasm_binary_vec,
        position_offset,
        0,
        lowered_signature_bytes,
        *types_section,
    )?;

    // Update the imports to point at the new lowered_signatures
    for imported_i64_function in imported_i64_functions.iter() {
        // Get the name length (module_len)
        let name_length_start_position = position_offset + imported_i64_function.position;
        let (import_module_name_length, import_module_name_length_byte_length) =
            read_bytes_as_varunit(
                wasm_binary_vec
                    .get(name_length_start_position..(name_length_start_position + 4))
                    .unwrap(),
            )?;

        // Get the field length (field_len)
        let field_length_start_position = name_length_start_position
            + import_module_name_length_byte_length
            + import_module_name_length as usize;
        let (import_field_name_length, import_field_name_length_byte_length) =
            read_bytes_as_varunit(
                wasm_binary_vec
                    .get(field_length_start_position..(field_length_start_position + 4))
                    .unwrap(),
            )?;

        // Get the function signature position (type)
        // +1 because of the external_kind (a single byte)
        let import_function_signature_position = field_length_start_position
            + import_field_name_length_byte_length
            + import_field_name_length as usize
            + 1;

        // Get the signature byte length (to remove later)
        let (import_function_signature, import_function_signature_byte_length) =
            read_bytes_as_varunit(
                &wasm_binary_vec
                    [import_function_signature_position..(import_function_signature_position + 4)],
            )?;

        // Change the signature index to our newly created import index
        let lowered_signature_vec_index = lowered_signatures
            .iter()
            .position(|x| x.original_signature_index == import_function_signature as usize)
            .unwrap();
        let new_signature_index = (type_signatures.len() + lowered_signature_vec_index) as u32;
        let new_signature_bytes = get_u32_as_bytes_for_varunit(new_signature_index);
        remove_number_of_bytes_in_vec_at_position(
            &mut wasm_binary_vec,
            import_function_signature_position,
            import_function_signature_byte_length,
        );
        insert_bytes_into_vec_at_position(
            &mut wasm_binary_vec,
            import_function_signature_position,
            new_signature_bytes.clone(),
        );

        let byte_length_difference =
            (new_signature_bytes.len() - import_function_signature_byte_length) as usize;
        position_offset += byte_length_difference;
    }

    // Add the signatures for the trampoline functions in the Functions section
    let functions_section = wasm_sections
        .iter()
        .find(|&x| x.code == WasmSectionCode::Function)
        .unwrap();
    let trampoline_signatures = trampoline_functions
        .iter()
        .cloned()
        .map(|tf| get_u32_as_bytes_for_varunit(tf.signature_index as u32))
        .collect::<Vec<_>>();
    position_offset += add_entries_to_section(
        wasm_binary_vec,
        position_offset,
        0,
        trampoline_signatures,
        *functions_section,
    )?;

    // Edit calls to the original function, to now point at the trampoline functions'
    // NOTE: Since Calls are a part of the function body, we need to calculate the offset
    // from modifying the calls, before adding the trampoline functions. Thus, we get an,
    // insertion_offset.
    let mut calls_byte_offset: usize = 0;
    for imported_i64_function in imported_i64_functions.iter() {
        for wasm_call_to_old_function in wasm_calls
            .iter()
            .filter(|&x| x.function_index == imported_i64_function.function_index)
        {
            // Get the old call
            let call_index_start_position =
                position_offset + calls_byte_offset + wasm_call_to_old_function.position + 1;
            let call_index_end_position =
                std::cmp::min(call_index_start_position + 4, wasm_binary_vec.len());

            let wasm_call_function_index_bytes = wasm_binary_vec
                .get(call_index_start_position..call_index_end_position)
                .unwrap();
            let (_, call_index_byte_length) =
                read_bytes_as_varunit(wasm_call_function_index_bytes)?;
            remove_number_of_bytes_in_vec_at_position(
                &mut wasm_binary_vec,
                call_index_start_position,
                call_index_byte_length,
            );

            let trampoline_function_vec_index = trampoline_functions
                .iter()
                .position(|x| x.signature_index == imported_i64_function.signature_index)
                .unwrap();
            let trampoline_function_index = wasm_functions.len() + trampoline_function_vec_index;
            let trampoline_function_bytes =
                get_u32_as_bytes_for_varunit(trampoline_function_index as u32);
            insert_bytes_into_vec_at_position(
                &mut wasm_binary_vec,
                call_index_start_position,
                trampoline_function_bytes.to_vec(),
            );

            let byte_length_difference =
                (trampoline_function_bytes.len() - call_index_byte_length) as usize;
            calls_byte_offset += byte_length_difference;

            // Also, we may need to update the function body size
            // If the function signature had a larger byte_length
            if byte_length_difference > 0 {
                // We need to subtract what we just added here, since the body size is BEFORE the call
                let function_size_position = position_offset + calls_byte_offset
                    - byte_length_difference
                    + wasm_call_to_old_function.function_body_position;

                let function_size_bytes = wasm_binary_vec
                    .get(function_size_position..(function_size_position + 4))
                    .unwrap();
                let (function_size, function_size_byte_length) =
                    read_bytes_as_varunit(function_size_bytes)?;
                remove_number_of_bytes_in_vec_at_position(
                    &mut wasm_binary_vec,
                    function_size_position,
                    function_size_byte_length,
                );

                let new_function_size = function_size + byte_length_difference as u32;
                let new_function_size_bytes =
                    get_u32_as_bytes_for_varunit(new_function_size as u32);
                insert_bytes_into_vec_at_position(
                    &mut wasm_binary_vec,
                    function_size_position,
                    new_function_size_bytes.to_vec(),
                );

                let function_size_byte_length_difference =
                    (new_function_size_bytes.len() - function_size_byte_length) as usize;
                calls_byte_offset += function_size_byte_length_difference;
            }
        }
    }

    // Add the trampoline functions to the code section
    let code_section = wasm_sections
        .iter()
        .find(|&x| x.code == WasmSectionCode::Code)
        .unwrap();

    let trampoline_function_bytes = trampoline_functions
        .iter()
        .cloned()
        .map(|tf| tf.bytes)
        .collect::<Vec<_>>();
    position_offset += add_entries_to_section(
        wasm_binary_vec,
        position_offset,
        calls_byte_offset,
        trampoline_function_bytes,
        *code_section,
    )?;

    //Done!
    return Ok(());
}

/// Function to add "entries" (E.g Types in the Type section),
/// to a section. And update it's count of entries, as well as length
/// Starting offset is the overall position offset for the start of the section
/// Insertion offset is the offset for the body of the section (Useful for sections like the
/// Code section, which need it's hader, body, and tail modified)
fn add_entries_to_section(
    wasm_binary_vec: &mut Vec<u8>,
    starting_offset: usize,
    insertion_offset: usize,
    entries: Vec<Vec<u8>>,
    section: WasmSection,
) -> Result<usize, &'static str> {
    // Position offset that is calculated while adding entries, and returned.
    // This is then added to the overall position offset.
    let mut position_offset: usize = 0;

    // Calculate how many bytes will be added to the end of the section
    let added_bytes_from_entries: usize = entries.iter().map(|e| e.len()).sum();
    position_offset += added_bytes_from_entries;

    // Section size
    let section_length_position = starting_offset + section.start_position + 1;
    let (section_length, section_length_byte_length) = read_bytes_as_varunit(
        wasm_binary_vec
            .get(section_length_position..(section_length_position + 4))
            .unwrap(),
    )?;
    let new_section_length =
        section_length + (insertion_offset as u32) + (added_bytes_from_entries as u32);
    let new_section_length_bytes = get_u32_as_bytes_for_varunit(new_section_length);
    let new_section_length_bytes_length = new_section_length_bytes.len();
    remove_number_of_bytes_in_vec_at_position(
        wasm_binary_vec,
        section_length_position,
        section_length_byte_length,
    );
    insert_bytes_into_vec_at_position(
        wasm_binary_vec,
        section_length_position,
        new_section_length_bytes,
    );

    let section_length_byte_length_difference =
        (new_section_length_bytes_length - section_length_byte_length) as usize;
    position_offset += section_length_byte_length_difference;

    // Number of Entries (AKA Count)
    let number_of_entries_position =
        starting_offset + section.start_position + 1 + section_length_byte_length;
    let (number_of_entries, number_of_entries_byte_length) = read_bytes_as_varunit(
        wasm_binary_vec
            .get(number_of_entries_position..(number_of_entries_position + 4))
            .unwrap(),
    )?;
    let new_number_of_entries = number_of_entries + entries.len() as u32;
    let new_number_of_entries_bytes = get_u32_as_bytes_for_varunit(new_number_of_entries);
    remove_number_of_bytes_in_vec_at_position(
        wasm_binary_vec,
        number_of_entries_position,
        number_of_entries_byte_length,
    );
    insert_bytes_into_vec_at_position(
        wasm_binary_vec,
        number_of_entries_position,
        new_number_of_entries_bytes.clone(),
    );

    let section_count_byte_length_difference =
        (number_of_entries_byte_length - new_number_of_entries_bytes.len()) as usize;
    position_offset += section_count_byte_length_difference;

    // Add the bytes of the entries
    // previous_entry_offset is the number of bytes added
    // byte inserting the previous entries (this is to make sure
    // entries are added in order).
    // TODO: This is O(n^2), if we need a speedup look here.
    let mut previous_entry_offset = 0;
    for entry in entries.iter() {
        for i in 0..entry.len() {
            wasm_binary_vec.insert(
                starting_offset
                    + section_length_byte_length_difference
                    + section_count_byte_length_difference
                    + insertion_offset
                    + section.end_position
                    + previous_entry_offset
                    + i,
                (*entry)[i],
            );
        }
        previous_entry_offset += entry.len();
    }

    position_offset += insertion_offset;
    return Ok(position_offset);
}

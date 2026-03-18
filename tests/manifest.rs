mod manifest_test {
    #[test]
    fn parses_embedded_drivers_toml() {
        let manifest = prinstall::drivers::manifest::Manifest::load_embedded();
        assert!(manifest.manufacturers.len() >= 8);
    }

    #[test]
    fn finds_manufacturer_by_model_prefix() {
        let manifest = prinstall::drivers::manifest::Manifest::load_embedded();
        let mfr = manifest.find_manufacturer("HP LaserJet Pro MFP M428fdw");
        assert!(mfr.is_some());
        assert_eq!(mfr.unwrap().name, "HP");
    }

    #[test]
    fn finds_manufacturer_case_insensitive_prefix() {
        let manifest = prinstall::drivers::manifest::Manifest::load_embedded();
        let mfr = manifest.find_manufacturer("RICOH IM C3000");
        assert!(mfr.is_some());
        assert_eq!(mfr.unwrap().name, "Ricoh");
    }

    #[test]
    fn returns_none_for_unknown_manufacturer() {
        let manifest = prinstall::drivers::manifest::Manifest::load_embedded();
        let mfr = manifest.find_manufacturer("Acme Printer 3000");
        assert!(mfr.is_none());
    }

    #[test]
    fn universal_drivers_present_for_hp() {
        let manifest = prinstall::drivers::manifest::Manifest::load_embedded();
        let mfr = manifest.find_manufacturer("HP LaserJet").unwrap();
        assert!(mfr.universal_drivers.len() >= 2);
        assert!(mfr.universal_drivers.iter().any(|d| d.name.contains("PCL6")));
    }
}

{
  description = "Control a desktop GUI from the command line, for AI agents";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = nixpkgs.lib.genAttrs systems;
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          manifest = (pkgs.lib.importTOML ./guictl/Cargo.toml).package;
        in
        rec {
          guictl = pkgs.rustPlatform.buildRustPackage {
            pname = manifest.name;
            version = manifest.version;

            src = ./.;
            cargoLock.lockFile = ./Cargo.lock;

            nativeBuildInputs = [ pkgs.makeWrapper ];

            # The tools guictl runs that a Plasma session doesn't already
            # provide. They go after the user's PATH, so a system copy wins.
            # kscreen-doctor and kbuildsycoca6 are left to the session, to
            # match the Plasma version that is actually running. Tesseract
            # comes with English only: every language together is about 1 GB.
            postInstall = ''
              wrapProgram $out/bin/guictl --suffix PATH : ${
                pkgs.lib.makeBinPath [
                  (pkgs.tesseract.override { enableLanguages = [ "eng" ]; })
                  pkgs.wl-clipboard
                  pkgs.pulseaudio
                  pkgs.xdg-utils
                ]
              }
            '';

            meta = {
              description = manifest.description;
              homepage = "https://github.com/RaresBaa/guictl";
              license = pkgs.lib.licenses.mit;
              mainProgram = "guictl";
              platforms = systems;
            };
          };
          default = guictl;
        }
      );
    };
}

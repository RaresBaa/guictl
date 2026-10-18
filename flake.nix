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
            #
            # Not plain `wrapProgram`: it hides the real binary behind
            # `.guictl-wrapped`, and NixOS's KWin carries a patch that
            # recognises exactly that `.<name>-wrapped` convention and maps
            # it back to `<name>` when deciding whether a caller may use
            # org.kde.KWin.ScreenShot2 - so `guictl setup`'s self-written
            # `.desktop` file, which points at its own resolved
            # `/proc/<pid>/exe` (the real `.guictl-wrapped` binary), never
            # matches on NixOS. Naming the real binary something that isn't
            # `.*-wrapped` sidesteps that special case entirely.
            postInstall = ''
              mv $out/bin/guictl $out/bin/guictl-unwrapped

              # `guictl setup` grants the ScreenShot2 permission by writing
              # a .desktop file to ~/.local/share/applications - but KWin
              # only trusts a .desktop entry that lives in a root-owned
              # XDG_DATA_DIRS directory, so that self-written one is never
              # authorized: KWin just returns NoAuthorized, with no consent
              # dialog and nothing in the journal, which looks exactly like
              # guictl itself is broken. Shipping an equivalent entry as
              # this package's own build output lands it in the Nix
              # profile's share/applications - root-owned, read-only - so
              # KWin authorizes it without `guictl setup` ever running.
              mkdir -p $out/share/applications
              cat > $out/share/applications/guictl.desktop <<DESKTOP
              [Desktop Entry]
              Type=Application
              Name=guictl
              Exec=$out/bin/guictl-unwrapped
              NoDisplay=true
              X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2
              DESKTOP

              makeWrapper $out/bin/guictl-unwrapped $out/bin/guictl --suffix PATH : ${
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

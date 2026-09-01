{
  description = "Generic Chrome EC Type-C mode selector";

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
        in
        {
          default = pkgs.rustPlatform.buildRustPackage {
            pname = "cros-typec-selector";
            version = "0.1.0";
            src = pkgs.lib.cleanSource self;
            cargoLock.lockFile = ./Cargo.lock;
            nativeBuildInputs = [ pkgs.pkg-config ];
            buildInputs = [ pkgs.systemd ];
            postInstall = ''
              install -Dm644 man/cros-typec-selector.8 $out/share/man/man8/cros-typec-selector.8
              install -Dm644 systemd/cros-typec-selector.service $out/lib/systemd/system/cros-typec-selector.service
              substituteInPlace $out/lib/systemd/system/cros-typec-selector.service \
                --replace-fail /usr/bin/cros-typec-selector $out/bin/cros-typec-selector
            '';
            meta = {
              description = "Generic Chrome EC Type-C mode selector";
              license = pkgs.lib.licenses.mit;
              platforms = pkgs.lib.platforms.linux;
              mainProgram = "cros-typec-selector";
            };
          };
        }
      );

      formatter = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        pkgs.writeShellApplication {
          name = "format-cros-typec-selector";
          runtimeInputs = [
            pkgs.cargo
            pkgs.rustfmt
            pkgs.nixfmt
          ];
          text = ''
            nixfmt flake.nix
            cargo fmt --all
          '';
        }
      );

      checks = forAllSystems (system: {
        package = self.packages.${system}.default;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
        in
        {
          default = pkgs.mkShell {
            packages = with pkgs; [
              cargo
              rustc
              rustfmt
              clippy
              nixfmt
              pkg-config
              systemd
              systemd.dev
            ];
            RUST_BACKTRACE = "1";
          };
        }
      );

      nixosModules.default =
        {
          config,
          lib,
          pkgs,
          ...
        }:
        let
          cfg = config.services.cros-typec-selector;
        in
        {
          options.services.cros-typec-selector = {
            enable = lib.mkEnableOption "generic Chrome EC Type-C mode selection";
            package = lib.mkOption {
              type = lib.types.package;
              default = self.packages.${pkgs.system}.default;
              defaultText = lib.literalExpression "inputs.cros-typec-selector.packages.\${pkgs.system}.default";
              description = "The cros-typec-selector package to run.";
            };
            writableSysfsPaths = lib.mkOption {
              type = lib.types.listOf lib.types.str;
              default = [ "/sys/class/typec" ];
              description = "Type-C sysfs paths writable by the service; use canonical /sys/devices paths if class symlink exceptions are insufficient.";
            };
          };
          config = lib.mkIf cfg.enable {
            systemd.services.cros-typec-selector = {
              description = "Chrome EC Type-C mode selector";
              wantedBy = [ "multi-user.target" ];
              after = [ "systemd-udevd.service" ];
              serviceConfig = {
                Type = "simple";
                ExecStart = "${lib.getExe cfg.package} daemon --live";
                Restart = "on-failure";
                RestartSec = "2s";
                NoNewPrivileges = true;
                ProtectSystem = "strict";
                ProtectHome = true;
                PrivateTmp = true;
                ProtectKernelTunables = true;
                ProtectKernelModules = true;
                ProtectControlGroups = true;
                RestrictAddressFamilies = [
                  "AF_NETLINK"
                  "AF_UNIX"
                ];
                ReadWritePaths = cfg.writableSysfsPaths;
                CapabilityBoundingSet = "";
                LockPersonality = true;
                MemoryDenyWriteExecute = true;
                RestrictRealtime = true;
                SystemCallArchitectures = "native";
              };
            };
          };
        };
    };
}

# 3D models

`mouse.omkm` is the mouse on the Keys & mouse page, drawn by `crates/app/src/gear.rs`.

It is built from **“Logitech G502 x lightspeed”** by **Okopchi**
(<https://sketchfab.com/3d-models/logitech-g502-x-lightspeed-cda7107cc1444b4788d747f0361f7d40>),
licensed under [Creative Commons Attribution 4.0](https://creativecommons.org/licenses/by/4.0/).
Changes: the logo and the feet are left out, the surface normals are smoothed, the shell is split into its
buttons (`mouse-regions.json`) and recoloured to match the drawn keyboard. The credit also shows in the
window, at the bottom of Advanced.

To build it again from the downloaded `.glb`:

```sh
python3 -I scripts/mesh-pack.py g502.glb crates/app/assets/gear/mouse.omkm \
  --size 130 --drop 'Material\.003$' --smooth 38 --regions crates/app/assets/gear/mouse-regions.json
```

`scripts/mesh-pack.py --list model.glb` shows a model's parts first. The file format is described at the top
of that script.

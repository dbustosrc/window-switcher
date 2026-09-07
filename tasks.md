# Plan de mejoras

Objetivo principal: mantener una interfaz simple y funcional, con la menor latencia posible al abrir y recorrer el selector, consumo mínimo en reposo y sin introducir regresiones.

Análisis posterior con mediciones manuales de CPU/RAM y estudio del selector nativo superpuesto: [analysis-performance-alt-tab-2026-08-27.md](analysis-performance-alt-tab-2026-08-27.md).

## Métricas objetivo

- [x] Medir latencia de la primera apertura del selector (p50 y p95).
- [x] Medir latencia de cada cambio de selección mientras Alt permanece presionado.
- [x] Medir CPU y memoria privada en reposo.
- [x] Medir objetos GDI, handles y memoria después de 500 ciclos de apertura/cierre.
- [ ] Registrar las mediciones antes y después de cada optimización importante.

## P0 — Seguridad para optimizar

### Pruebas de comportamiento

- [ ] Cubrir el orden de ventanas y la selección inicial normal/inversa.
- [ ] Cubrir Alt, Shift+Alt, Escape, flechas y liberación del modificador.
- [ ] Cubrir ventanas que se cierran durante una sesión de Alt+Tab.
- [ ] Cubrir ventanas minimizadas, elevadas, topmost y de otros escritorios virtuales.
- [x] Verificar que no aumenten los objetos GDI, HICON o HANDLE tras ciclos repetidos.

### Instrumentación de rendimiento

- [x] Añadir mediciones opcionales para enumeración, resolución de iconos y pintura.
- [x] Mantener la instrumentación desactivada en builds release normales.
- [ ] Crear un escenario reproducible con muchas ventanas y varios navegadores.

## P1 — Mayor impacto en responsividad

### Caché gráfica durante la sesión

- [x] Renderizar la tira estática de iconos una sola vez al abrir el selector.
- [x] En cada Tab, actualizar solamente el indicador de selección y el título.
- [x] Reutilizar DC, bitmap y contexto GDI+ mientras Alt permanezca presionado.
- [x] Liberar los buffers al cerrar el selector para no aumentar el consumo en reposo.
- [x] Invalidar la caché si cambia el conjunto de ventanas, DPI, monitor o tema.

Criterio de aceptación: recorrer rápidamente el selector no debe recrear ni reescalar todos los iconos en cada paso.

### Carga de iconos no bloqueante

- [x] Evitar esperas de hasta 250 ms por cada llamada a `WM_GETICON`.
- [x] Reducir los timeouts síncronos a un máximo razonable de 30–50 ms.
- [x] Mostrar inmediatamente un icono disponible o provisional.
- [x] Resolver iconos especiales fuera del hilo de interfaz y repintar solo si cambian.
- [x] Evitar dormir después del último intento de `SHGetFileInfoW`.
- [x] Mantener un único trabajo pendiente por clave de icono.

Criterio de aceptación: una aplicación lenta o bloqueada no debe retrasar perceptiblemente la aparición del selector.

### Caché de metadatos de procesos y ventanas

- [x] Mantener una caché limitada de ruta y elevación por PID.
- [x] Protegerse contra reutilización de PID usando el tiempo de creación del proceso.
- [x] Mantener una caché limitada de AUMID por HWND.
- [x] Invalidar entradas cuando desaparezca la ventana o termine el proceso.
- [x] Compartir esta caché con el observador de ventana en primer plano.

Criterio de aceptación: aperturas consecutivas no deben volver a abrir todos los procesos ni consultar propiedades que no hayan cambiado.

### Resolución diferida de Chrome, Edge y PWA

- [x] Separar la ruta del ejecutable de la clave usada para escoger el icono.
- [x] Consultar `IPropertyStore` solamente cuando falte información en caché.
- [x] Evitar resolver AUMID para cada ventana de navegador en cada Alt+Tab.
- [ ] Conservar los iconos diferenciados de perfiles y aplicaciones web.

### Coalescencia de pulsaciones

- [x] Acumular el desplazamiento de varias pulsaciones rápidas de Tab.
- [x] Mantener como máximo un mensaje de actualización pendiente en la cola.
- [x] Pintar directamente el índice más reciente.
- [x] Garantizar que la liberación de Alt se procese después del último desplazamiento.
- [ ] Probar repetición automática del teclado y cambios rápidos de dirección.

Criterio de aceptación: no debe existir una cola visible de selecciones atrasadas cuando el usuario avanza rápidamente.

### Enumerador específico para Alt+Backtick

- [x] Identificar primero el proceso/AUMID de la ventana activa.
- [x] Procesar únicamente las ventanas candidatas de esa aplicación.
- [x] Mantener el orden actual y el comportamiento de caché mientras Alt esté presionado.
- [ ] Verificar Chrome, Edge, PWA y aplicaciones con procesos auxiliares.

## P2 — Menos trabajo y asignaciones

### Cachear tema y configuración del sistema

- [x] Leer el tema una vez en lugar de consultar el Registro en cada repintado.
- [x] Resolver `only_current_desktop` una vez al cargar la configuración.
- [x] Actualizar los valores al recibir cambios relevantes del sistema.
- [x] Abrir claves del Registro con permisos de solo lectura.

### Reducir trabajo en el hook global de teclado

- [x] Retirar el `Mutex<Vec<HotKeyState>>` de la ruta de cada pulsación.
- [x] Representar el estado pequeño mediante bits, atomics o estado del hilo de mensajes.
- [x] Aplicar retorno temprano a teclas no relevantes.
- [x] Desactivar el logging detallado de cada tecla en releases normales.
- [x] Confirmar que el callback siempre retorna rápidamente.

### Consolidar el modelo de ventana

- [x] Reemplazar los vectores paralelos de aplicaciones y títulos por `WindowEntry`.
- [x] Reutilizar la capacidad de los vectores entre sesiones.
- [x] Reducir clones de rutas y títulos durante la enumeración.
- [x] Evitar construir colecciones intermedias que contienen la misma información.

### Optimizar la relación propietario/ventana

- [x] Construir un mapa de propietario a ventana durante `EnumWindows`.
- [x] Eliminar la búsqueda lineal repetida en `owner_hwnds`.
- [x] Conservar el manejo actual de `ApplicationFrameHost` y ventanas delegadas.

### Temporizador para reintentar el icono de bandeja

- [x] Sustituir el hilo que duerme tres segundos por `SetTimer`.
- [x] Garantizar que solo exista un reintento pendiente.
- [x] Cancelar el temporizador cuando el icono se registre correctamente.

## P3 — Robustez y compatibilidad

### Eliminar estado global mutable inseguro

- [x] Sustituir los `static mut` del teclado por estado seguro del hilo o atomics.
- [x] Sustituir el estado global del observador de primer plano por `AtomicBool`.
- [x] Limitar la vida de los préstamos obtenidos desde `GWLP_USERDATA`.

### DPI por monitor

- [x] Migrar a `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`.
- [x] Calcular el DPI del monitor donde se mostrará el selector.
- [x] Recrear solamente fuente y recursos dependientes del DPI cuando cambie.
- [ ] Probar movimiento entre monitores con escalas diferentes.

### Filtros de ventanas

- [x] Revisar la exclusión general de ventanas `TOPMOST`.
- [x] Aproximar las reglas del Alt+Tab clásico sin mostrar ventanas auxiliares.
- [ ] Verificar Task Manager con y sin “Siempre visible”.
- [x] Validar nuevamente el HWND antes de activar una selección.
- [x] Saltar de forma segura ventanas cerradas durante la sesión.

### Aplicaciones elevadas

- [x] Documentar claramente la limitación de integridad del hook global.
- [x] Evaluar un ejecutable firmado con `uiAccess` o un helper elevado mínimo.
- [x] Mantener como alternativa el inicio mediante tarea programada elevada.

### Muchas ventanas

- [x] Establecer un tamaño mínimo de icono seguro.
- [x] Evitar dimensiones cero o negativas en el layout.
- [x] Evaluar paginación o cuadrícula cuando no quepan todas las ventanas.

Decisión: se conserva la tira horizontal y se protegen todas las dimensiones con mínimos seguros. No se añade cuadrícula en esta fase porque cambiaría la navegación y la interfaz; si el número de ventanas hace que los iconos dejen de ser legibles, la cuadrícula debe tratarse como una mejora funcional independiente.

## Optimizaciones completadas

### Limpieza de código legacy

- [x] Eliminar el mensaje `WM_USER_REGISTER_TRAYICON`, sin publicadores desde la migración a `SetTimer`.
- [x] Eliminar las rutas antiguas `is_process_elevated` e `is_elevated`; la elevación usa ahora la caché centralizada de metadatos.
- [x] Integrar el helper de versión de Windows, usado una sola vez, directamente en `is_win11`.
- [x] Sustituir la dependencia `once_cell` por `std::sync::OnceLock`.
- [x] Eliminar la dependencia `indexmap` no utilizada por `inspect-windows`.
- [x] Eliminar implementaciones inseguras `Send`/`Sync` innecesarias de `SingleInstance`.
- [x] Eliminar la supresión `#[allow(unused)]` obsoleta de `check_error`.
- [x] Auditar funciones, tipos, constantes y dependencias públicas sin consumidores.

### Diagnóstico de fallos

- [x] Habilitar `window-switcher.log` de forma predeterminada.
- [x] Registrar arranque, PID, versión y cierre normal.
- [x] Guardar errores fatales y pánicos de cualquier hilo en `window-switcher-crash.log`.
- [x] Incluir ubicación del pánico y backtrace sin depender de que el INI cargue correctamente.
- [x] Detectar en el siguiente arranque sesiones que terminaron sin un cierre normal.

### Última validación — 2026-08-01

#### Benchmark end-to-end de Alt+Tab

- Escenario: 20 ciclos de calentamiento, 500 ciclos medidos y 3 cambios adicionales por apertura.
- Apertura fría: `17.907 ms` en release normal (`32.651 ms` con instrumentación y logging).
- Apertura caliente end-to-end: mínimo `19.554 ms`, p50 `32.425 ms`, p95 `40.683 ms`, p99 `45.084 ms`, máximo `53.752 ms`.
- Cierre end-to-end: p50 `2.501 ms`, p95 `3.692 ms`, p99 `4.427 ms`, máximo `6.387 ms`.
- Cambio de selección interno: p50 `3.154 ms`, p95 `4.327 ms`, p99 `4.923 ms`, máximo `7.335 ms`.
- Enumeración de ventanas: p50 `7.886 ms`, p95 `18.132 ms`, p99 `19.801 ms`, máximo `22.175 ms`.
- Creación de sesión gráfica: p50 `11.817 ms`, p95 `18.192 ms`, p99 `21.102 ms`, máximo `23.419 ms`.
- Después de 500 ciclos en release normal: handles `297 -> 297`, GDI `68 -> 68`, USER/HICON `22 -> 22`, memoria privada `9.281 -> 9.906 MiB`, working set `30.844 -> 31.449 MiB`.
- Prueba prolongada de 1.500 ciclos: GDI y USER sin cambios; memoria privada `+0.531 MiB`, confirmando estabilización y no crecimiento lineal. Handles `+3` por nuevas entradas vivas de la caché acotada de procesos durante la ejecución.
- CPU del proceso durante 500 ciclos y 2.000 selecciones: `11.281 s` acumulados en `30.1 s` de carga continua; este escenario es deliberadamente extremo y no representa el consumo en reposo.

- Release instrumentado: compilación correcta.
- Enumeración de 20 ventanas, 200 iteraciones: primera `0.922 ms`, p50 `0.110 ms`, p95 `0.192 ms`, máximo `0.750 ms` después del calentamiento.
- Herramienta de enumeración después del calentamiento: `134 -> 134` handles y `0 -> 0` objetos GDI.
- Instancia release en reposo: aproximadamente `4.0 MiB` privados, `17.3 MiB` de working set, `154` handles y 2 hilos.
- Prueba real de Alt+Tab: el hook propio cambió correctamente desde Explorador de archivos a la siguiente ventana; CPU acumulada y handles permanecieron estables tras el ciclo.
- Prueba automatizada de 500 sesiones gráficas: sin crecimiento de objetos GDI.
- `cargo test --workspace`: 8 pruebas correctas.
- `cargo clippy --workspace --all-targets --features perf -- -D warnings`: sin advertencias.

- [x] Alt+Tab muestra cada ventana individualmente sin agrupar por aplicación.
- [x] Alt+Backtick conserva el cambio entre ventanas de la misma aplicación.
- [x] Liberación RAII de handles de procesos.
- [x] Caché por PID durante cada enumeración.
- [x] Liberación correcta de bitmaps, DC, iconos y recursos GDI+.
- [x] El hook publica acciones mediante `PostMessageW`.
- [x] Reutilización de la lista de Alt+Backtick mientras el modificador está presionado.
- [x] Caché y poda de iconos por aplicación.
- [x] Supersampling reducido de 6× a 2×.
- [x] Título de ventana centrado con elipsis.
- [x] Texto renderizado con GDI+ y `AntiAliasGridFit`.
- [x] Fuente `Segoe UI Semibold` reutilizada durante toda la ejecución.
- [x] Build release, pruebas y Clippy validados después de los cambios.

## Orden recomendado de implementación

1. Pruebas e instrumentación.
2. Caché gráfica durante la sesión de Alt.
3. Carga de iconos no bloqueante.
4. Caché de procesos, ventanas y AUMID.
5. Caché de tema y configuración del sistema.
6. Coalescencia de pulsaciones.
7. Enumerador especializado para Alt+Backtick.
8. Optimización del hook global.
9. DPI por monitor y revisión de filtros.

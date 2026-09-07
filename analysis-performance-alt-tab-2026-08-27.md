# Análisis de rendimiento y superposición del selector nativo

Fecha de captura: 2026-08-27  
Versión de la aplicación: 1.19.0 con los cambios locales del repositorio  
Equipo de prueba: Windows, 16 procesadores lógicos  
Estado del código durante las mediciones: compilación `release`, aplicación ejecutada elevada mediante la tarea `WindowSwitcher`

Este documento conserva las mediciones y las hipótesis técnicas para poder repetir el análisis posteriormente. No documenta una corrección ya aprobada: las mejoras y soluciones enumeradas son candidatas que deben probarse de forma aislada y revertirse si no resultan efectivas.

## Resumen ejecutivo

- No se encontró evidencia de una fuga de memoria progresiva.
- La aplicación recién iniciada consume aproximadamente `3.9 MiB` privados y `16.5 MiB` de working set.
- Después de uso real, la memoria privada se estabiliza alrededor de `10–11 MiB`.
- La CPU en reposo fue `0.00 %` durante 20 segundos.
- Recorrer 50 selecciones dentro de una sola sesión utilizó `0.3438 s` de CPU, aproximadamente `6.88 ms` por selección.
- Abrir y cerrar 30 sesiones completas utilizó `1.2031 s` de CPU, aproximadamente `40.10 ms` por sesión.
- El filtro de tamaño basado en DWM no conserva memoria; su costo medido de enumeración fue p50 `0.435 ms` y p95 `1.738 ms` para 22 ventanas.
- La mayor diferencia entre el estado frío y caliente corresponde a cachés de procesos, AUMID e iconos, inicialización Shell/COM/GDI+ y memoria retenida por el asignador después del primer uso.
- Se observó una sola vez el selector nativo de Windows superpuesto al selector propio. La evidencia indica que un único `Tab` fue entregado a Windows; las pulsaciones siguientes sí fueron interceptadas. No volvió a reproducirse al repetir inmediatamente la misma prueba.

## Mediciones históricas de referencia

Fuente: `tasks.md`, validación del 2026-08-01.

### Reposo

- Memoria privada: aproximadamente `4.0 MiB`.
- Working set: aproximadamente `17.3 MiB`.
- Handles: `154`.
- Hilos: `2`.

### Benchmark end-to-end anterior

Escenario: 20 ciclos de calentamiento, 500 ciclos medidos y 3 pulsaciones adicionales de `Tab` por apertura.

- Memoria privada: `9.281 -> 9.906 MiB`.
- Working set: `30.844 -> 31.449 MiB`.
- Handles: `297 -> 297`.
- Objetos GDI: `68 -> 68`.
- Objetos USER/HICON: `22 -> 22`.
- CPU: `11.281 s` durante 500 aperturas y 2.000 selecciones, en `30.1 s` de carga continua.
- En 1.500 ciclos, la memoria privada aumentó `0.531 MiB` y luego se estabilizó; GDI y USER no aumentaron.

Estas cifras son importantes porque la memoria caliente actual está dentro del mismo rango que la versión anterior al cambio de geometría.

## Mediciones actuales

### Instancia caliente antes de reiniciar

Muestra de 20 segundos:

- Memoria privada: `10.73 MiB`, sin variación.
- Working set: promedio `29.036 MiB`, máximo `29.078 MiB`.
- Handles: `275`, sin variación.
- Hilos: `3`, sin variación.
- CPU: `0.00 %`.

### Instancia recién reiniciada

- Memoria privada: `3.906 MiB`.
- Working set: `16.473–16.500 MiB`.
- Handles: `137–139`.
- Hilos: `2`.

La instancia fría coincide con la referencia histórica. Esto descarta que el cambio reciente de `GetWindowPlacement` a límites visuales reserve varios megabytes al arrancar.

### Prueba 1: una sesión, 50 pulsaciones individuales de Tab

Procedimiento: mantener `Alt`, pulsar y soltar `Tab` 50 veces y finalmente soltar `Alt`.

- Duración total de la ventana de medición: `43.423 s`.
- CPU del proceso: `0.3438 s`.
- CPU por selección: aproximadamente `6.88 ms`.
- Promedio durante toda la ventana: `0.0495 %` del equipo o `0.792 %` de un núcleo.
- Memoria privada: `8.996 -> 10.488 MiB`, delta `+1.492 MiB`.
- Working set: `3.004 -> 17.004 MiB`.
- Pico de working set: `23.375 -> 23.375 MiB`, sin aumento.
- Handles inmediatos: `279 -> 281`.
- Hilos inmediatos: `2 -> 3`.

La cifra inicial de working set no es comparable como asignación nueva: Windows había recortado el conjunto residente a solo `3 MiB`. El pico no aumentó. Quince segundos después, la CPU fue `0.00 %`, la memoria privada continuó en `10.488 MiB` y el tercer hilo permanecía dormido. Más tarde el proceso volvió a 2 hilos y los handles bajaron, coherente con la finalización del cargador asíncrono de iconos.

Durante esta prueba ocurrió una vez la superposición del selector nativo descrita más adelante.

### Prueba 2: 30 sesiones completas de Alt+Tab

Procedimiento: pulsar y soltar completamente `Alt+Tab` 30 veces.

- Duración total de la ventana de medición: `56.073 s`.
- CPU del proceso: `1.2031 s`.
- CPU por sesión: aproximadamente `40.10 ms`.
- Promedio durante toda la ventana: `0.1341 %` del equipo o `2.146 %` de un núcleo.
- Memoria privada: `10.832 -> 10.281 MiB`, delta `-0.551 MiB`.
- Working set: `7.824 -> 7.301 MiB`, delta `-0.523 MiB`.
- Pico de working set: `23.375 -> 23.375 MiB`, sin aumento.
- Handles: `276 -> 276`.
- Hilos: `2 -> 2`.

Abrir una sesión completa es más costoso que cambiar la selección dentro de una sesión porque vuelve a enumerar ventanas, prepara entradas/iconos y crea los buffers gráficos. No hubo crecimiento de recursos a través de las 30 sesiones.

### Enumeración del filtro actual

Escenario: 22 ventanas, 200 enumeraciones con el ejecutable `inspect-windows`.

- Primera enumeración: `7.592 ms`.
- p50: `0.435 ms`.
- p95: `1.738 ms`.
- Máximo: `3.335 ms`.
- Handles: `144 -> 144`.
- Objetos GDI: `0 -> 0`.

La referencia histórica de enumeración era p50 `0.110 ms` y p95 `0.192 ms`. La consulta `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)` aumentó el costo relativo, pero el costo absoluto continúa por debajo de 2 ms en el percentil 95 y no trabaja en reposo.

## Limitaciones de la metodología

- La duración total de las pruebas manuales incluye el tiempo para leer la indicación y responder. Por eso son más útiles los segundos totales de CPU y el costo por operación que el porcentaje promedio durante toda la ventana.
- El benchmark automático no pudo lanzar directamente el ejecutable instalado porque requiere elevación (`error 740`).
- Se intentaron 100 eventos mediante una herramienta de automatización, pero el hook elevado no recorrió la ruta normal. La memoria permaneció en `3.906 MiB` y la CPU fue casi nula; esos eventos se descartaron como benchmark funcional.
- La prueba manual actual tenía más ventanas y un conjunto distinto de aplicaciones que el benchmark histórico. Los costos por apertura no constituyen todavía una comparación A/B estricta.

## Análisis de memoria

### 1. Caché de procesos

Ubicación: `src/utils/window.rs`.

- `PROCESS_CACHE_LIMIT = 128`.
- Cada entrada mantiene un `HANDLE` abierto, ruta del módulo, elevación, tiempo de creación y marca de último uso.
- El handle permite validar rápidamente que el PID continúa vivo y evita repetir `OpenProcess`, la consulta de ruta y la consulta del token en cada Alt+Tab.
- La diferencia aproximada entre 137 handles en frío y 275–283 en caliente coincide con una caché de procesos ampliamente poblada.
- La caché elimina procesos terminados cuando vuelve a consultarse y aplica LRU al alcanzar el límite, pero no elimina procesos vivos que ya no tienen una ventana candidata.

Es la principal candidata para reducir handles y una parte del estado caliente. No explica por sí sola varios megabytes de memoria privada; el costo de cada entrada es pequeño y parte del costo de los handles reside en el kernel.

### 2. Caché AUMID

Ubicación: `src/utils/window.rs`.

- `AUMID_CACHE_LIMIT = 256`.
- Clave: HWND, PID y tiempo de creación del proceso.
- Se poda según los HWND que continúan enumerados.
- Conserva cadenas pequeñas y resultados opcionales; no parece ser el principal consumidor.

### 3. Caché de iconos

Ubicación: `src/app.rs`, campo `cached_icons`.

- Conserva un `HICON` por clave activa de aplicación/perfil/PWA.
- Elimina y destruye iconos cuando su clave ya no aparece entre las ventanas activas.
- Evita recargar Shell, archivos AppX, perfiles y PWA en cada apertura.
- Explica parte de la memoria caliente y es importante para la responsividad.

### 4. Cargador asíncrono de iconos

Ubicaciones: `src/app.rs::start_icon_jobs` y `src/utils/app_icon.rs`.

- Crea un hilo con pila reservada de 256 KiB para cada lote nuevo.
- `pending_icon_jobs` evita duplicar el mismo trabajo, pero lotes diferentes pueden crear hilos separados.
- La resolución puede consultar Shell, archivos AppX/PWA y `WM_GETICON` con timeouts de 40 ms.
- `SHGetFileInfoW` puede reintentarse tres veces con dos esperas de 30 ms.
- En la prueba 1 apareció temporalmente un tercer hilo y varios handles; posteriormente desaparecieron, por lo que la evidencia favorece trabajo transitorio y no fuga.

### 5. Sesión gráfica

Ubicación: `src/painter.rs`.

- `PaintSession` contiene DC, bitmaps, imágenes GDI+, brush y versiones seleccionadas de iconos.
- `GdiAAPainter::unpaint` elimina la sesión completa al cerrar el selector.
- Las implementaciones `Drop` liberan GDI+, bitmaps y DC.
- Las pruebas automatizadas existentes no muestran crecimiento de objetos GDI.
- `window_buffer` conserva capacidad del vector entre sesiones, pero sus cadenas se destruyen al limpiar y el bloque retenido es pequeño.

### 6. Estado del runtime y working set

Después del primer uso, COM, Shell, GDI+, fuentes y el asignador pueden conservar páginas comprometidas aunque los objetos funcionales ya se hayan destruido. Windows puede mantener o recortar esas páginas del working set. Esto explica por qué se observaron valores entre 3 y 29 MiB sin que cambiara la memoria privada de forma equivalente.

## Posibles mejoras de recursos y rendimiento

Las siguientes propuestas no están implementadas. Deben probarse individualmente y compararse con las métricas de este documento.

### Prioridad 1: podar la caché de procesos por procesos realmente candidatos

Al terminar cada enumeración ya se conoce el conjunto de PID que aporta ventanas. La caché podría conservar únicamente esos PID, además de eliminar procesos terminados.

Beneficios esperados:

- Menos handles después de cerrar ventanas o aplicaciones que permanecen vivas en segundo plano.
- Menor estado caliente sin introducir un temporizador ni actividad periódica.

Riesgo:

- Si una aplicación pierde temporalmente todas sus ventanas y vuelve a mostrar una, habrá que consultar nuevamente sus metadatos.

### Prioridad 2: reducir o aplicar TTL al cache de handles

Opciones para un experimento A/B:

1. Reducir el límite de 128 a 32 o 64.
2. Conservar metadatos, pero cerrar el handle después de un periodo de inactividad.
3. Cerrar todos los handles al finalizar una sesión y conservar solo ruta/elevación/tiempo de creación, revalidando el PID al reutilizar la entrada.

Debe medirse el impacto en p50/p95 de apertura. La opción 1 es la prueba menos invasiva; la opción 3 reduce más recursos, pero sacrifica parte del beneficio de la caché.

### Prioridad 3: limitar lotes simultáneos del cargador de iconos

Mantener como máximo un hilo de carga bajo demanda y dejarlo finalizar al vaciar la cola. Esto evitaría hilos solapados sin conservar permanentemente un worker en reposo.

No se recomienda un hilo residente permanente si la prioridad es el mínimo de recursos en reposo.

### Prioridad 4: usar GetWindowRect como ruta rápida del filtro

El umbral de ventana pequeña es aproximado (`120 x 90`), por lo que normalmente no necesita la precisión de los bordes DWM. Se puede comparar:

1. `GetWindowRect` como primera opción para ventanas visibles no minimizadas.
2. `DwmGetWindowAttribute` solo si `GetWindowRect` falla o devuelve una geometría dudosa.
3. `rcNormalPosition` únicamente como respaldo y nunca para excluir una ventana minimizada legítima.

Objetivo: acercar la enumeración al p50 histórico sin volver a excluir Eclipse. Debe validarse con Eclipse maximizado/restaurado, ventanas DPI escaladas y ventanas con bordes invisibles.

### Prioridad 5: instrumentar tamaños de caché en builds de diagnóstico

Registrar únicamente bajo la característica `perf`:

- Número de entradas de `PROCESS_CACHE`.
- Número de handles de proceso retenidos.
- Número de entradas AUMID.
- Número de iconos y trabajos pendientes.
- Memoria privada y handles antes/después de una sesión.

No debe escribirse al log desde el callback del teclado.

### Mejoras no recomendadas inicialmente

- Eliminar todas las cachés: reduce el estado caliente, pero perjudica la responsividad y aumenta llamadas Shell/COM.
- Liberar e inicializar GDI+ en cada Alt+Tab: mejora la cifra fría a costa de la primera apertura.
- Ejecutar temporizadores periódicos solo para reducir memoria: añade actividad en reposo y contradice el objetivo de CPU mínima.

## Bug: selector nativo superpuesto

### Secuencia observada

1. Se mantuvo `Alt` presionado.
2. Se pulsó y soltó `Tab` repetidamente.
3. Aproximadamente en la pulsación 15 apareció el selector nativo de Windows por encima del selector propio.
4. El selector propio continuó visible debajo.
5. Las pulsaciones posteriores de `Tab` continuaron recorriendo el selector propio, pero el selector nativo no avanzó.
6. Al soltar `Alt`, ambos selectores desaparecieron.
7. La misma prueba se repitió inmediatamente y el problema no volvió a aparecer.

### Inferencia con confianza media

La secuencia es compatible con un solo evento `Tab` entregado a Windows:

- Ese evento abrió el selector nativo.
- Los eventos siguientes sí fueron interceptados y devueltos como `LRESULT(1)`, por eso el selector nativo no cambió de selección.
- El evento físico de liberación de `Alt` llegó tanto a nuestra lógica como a Windows y cerró ambos selectores.

No hay evidencia de que Windows eliminara permanentemente el hook. Si eso hubiera ocurrido, todas las pulsaciones siguientes habrían recorrido el selector nativo y las pruebas posteriores también habrían fallado.

### Ruta de código relevante

Ubicación principal: `src/keyboard.rs::keyboard_proc`.

- El hook `WH_KEYBOARD_LL` se instala en el mismo hilo que ejecuta después el bucle principal y el trabajo de enumeración/pintura.
- El estado del modificador se conserva en `thread_local RefCell<Vec<HotKeyState>>`.
- Una pulsación reconocida publica trabajo mediante `PostMessageW` y retorna `LRESULT(1)`.
- La coalescencia usa `PENDING_APP_DELTA` y `APP_MESSAGE_PENDING`; puede perder una actualización visual si existiera un error lógico, pero no debería dejar pasar el evento físico porque el callback retorna 1 independientemente de la cola.
- Los eventos `Tab` de liberación actualmente se envían a `CallNextHookEx`; solo se suprime el key-down reconocido.
- `mask_alt_menu` llama a `SendInput` dentro del callback para inyectar `Ctrl` down/up al iniciar la sesión. Se añadió para mitigar el menú de SAP y es la única inyección de teclado dentro del hook.
- El callback no realiza un retorno temprano explícito para `code < 0`, aunque la convención de hooks exige no procesar esos eventos.
- Alt izquierdo y derecho se representan con el mismo scan code `0x38`; no se utiliza `LLKHF_EXTENDED` para mantener un estado independiente.

### Hipótesis priorizadas

#### H1. Deriva transitoria del estado interno del modificador

Si `is_modifier_pressed` se vuelve falso mientras `IS_SWITCHING_APPS` sigue activo, el siguiente Tab cae hasta `CallNextHookEx`. La sesión propia permanece visible porque su estado global sigue activo. Esta hipótesis coincide bien con un único evento escapado.

#### H2. Interacción de SendInput dentro del callback

El toque neutral de Ctrl genera eventos inyectados mientras Alt continúa físicamente presionado. Aunque no coincide directamente con Tab, puede interactuar con el estado del teclado, otros hooks globales o el shell. Es una diferencia relevante frente al código original y debe aislarse experimentalmente.

#### H3. El hook comparte el hilo con enumeración y pintura

Los callbacks de `WH_KEYBOARD_LL` son entregados al hilo instalador. Durante una apertura, ese mismo hilo enumera ventanas y crea la sesión gráfica. Las mediciones actuales están muy por debajo del timeout de Windows, por lo que esta no es la explicación más probable del evento único, pero separar el hook mejoraría la robustez bajo aplicaciones lentas o cargas imprevistas.

#### H4. Evento especial no tratado correctamente

El callback procesa `KBDLLHOOKSTRUCT` sin verificar primero `code < 0` y no registra `wParam`, `vkCode`, flags de inyección ni `LLKHF_EXTENDED`. Sin esos datos no es posible saber si el evento escapado tenía una forma distinta de los eventos normales.

#### H5. Interferencia externa en la cadena de hooks

Otra aplicación con un hook global podría alterar orden o estado. Es posible, pero actualmente no hay evidencia para atribuirle la causa.

### Instrumentación recomendada antes de corregir

La próxima vez que se investigue el bug, el primer cambio debe ser diagnóstico y temporal:

1. Añadir un ring buffer fijo, sin asignaciones ni I/O, con los últimos 128–256 eventos.
2. Guardar por evento: contador, timestamp, `code`, `wParam`, `vkCode`, `scanCode`, flags, estado de Alt izquierdo/derecho, `IS_SWITCHING_APPS`, acción elegida y valor de retorno.
3. Medir duración del callback y contar fallos de `PostMessageW`.
4. Detectar `EVENT_SYSTEM_SWITCHSTART`/`EVENT_SYSTEM_SWITCHEND` mediante `SetWinEventHook`.
5. Si Windows inicia su selector mientras el selector propio está activo, publicar un mensaje al hilo principal y volcar allí el ring buffer al log.
6. Mantener la instrumentación desactivada en release normal y no escribir nunca al disco desde `keyboard_proc`.

Esto permitiría distinguir estado interno incorrecto, evento inyectado, callback omitido y evento no reconocido.

## Posibles soluciones al bug

No implementar varias a la vez. Cada opción debe probarse, y retirarse antes de continuar si no resuelve el problema.

### Solución candidata A: contención mínima durante una sesión activa

Una vez que `IS_SWITCHING_APPS` sea verdadero:

- Suprimir todo `Tab` down y up hasta la liberación final de Alt, aunque el estado derivado de `HotKeyState` se haya desincronizado.
- Procesar el desplazamiento solamente en key-down.
- Mantener Alt izquierdo y derecho mediante un bitmask que considere `LLKHF_EXTENDED` y cerrar solo cuando ambos estén liberados.
- Para `code < 0`, llamar inmediatamente a `CallNextHookEx` sin tocar el estado.

Esta es la corrección mínima más alineada con la evidencia de un estado transitorio. No protege contra un evento que nunca llegue al hook, pero sí contra una falsa pérdida del modificador dentro de nuestra lógica.

Nota histórica: anteriormente se probó consumir Tab-up junto con `WM_CANCELMODE` para el problema del menú SAP y el conjunto no fue efectivo. Eso no evaluó de forma aislada esta contención para la superposición nativa.

### Solución candidata B: hilo dedicado para el hook

Instalar `WH_KEYBOARD_LL` en un hilo dedicado con su propio message loop y una pila pequeña. El callback debe limitarse a:

- Actualizar una máquina de estados compacta.
- Suprimir inmediatamente las teclas relevantes.
- Acumular el delta y publicar mensajes al hilo de interfaz.

Enumeración, Shell, pintura, logging e inyección de teclado quedarían fuera del hilo del hook.

Ventaja: máxima robustez frente a latencia del hilo gráfico.  
Costo: un hilo y un handle permanentes; debe cuantificarse porque el objetivo también es minimizar recursos.

### Solución candidata C: retirar SendInput del callback

Probar de forma aislada una versión en la que `mask_alt_menu`:

1. Se ejecute fuera del callback, mediante un mensaje al hilo principal; o
2. Se desactive temporalmente para una comparación A/B.

Debe repetirse tanto la prueba rápida de SAP como una prueba prolongada de Alt+Tab. Si vuelve el menú SAP o no cambia la superposición, revertir antes del siguiente experimento.

### Solución candidata D: supresión simétrica de Tab

Suprimir key-down y key-up de Tab mientras la sesión propia esté activa. Esto evita entregar al shell secuencias incompletas. Por sí sola no impide que Windows actúe si un key-down completo no llegó al hook, por lo que se considera complemento de A, no solución raíz demostrada.

### Solución no recomendada: cancelar el selector nativo con más teclas sintéticas

Enviar Escape, Alt-up u otras teclas al detectar el selector nativo puede dejar modificadores lógicamente atascados. Ya se observó anteriormente un experimento que dejó el teclado en un estado confuso. La detección de `SWITCHSTART` debe usarse primero para diagnóstico, no como mecanismo automático de recuperación.

## Orden recomendado para el próximo análisis

1. Añadir instrumentación temporal con ring buffer y detección de `SWITCHSTART`.
2. Repetir 10 sesiones de 100 pulsaciones individuales de Tab y registrar si ocurre la superposición.
3. Si se confirma deriva del estado, probar solo la solución A.
4. Si el evento no llega al hook o hay latencias anómalas, probar solo la solución B.
5. Si los eventos inyectados aparecen cerca del fallo, comparar de forma aislada la solución C, conservando la prueba de SAP.
6. Ejecutar las dos pruebas manuales de rendimiento de este documento y comparar CPU, memoria, handles e hilos.
7. Retirar la instrumentación temporal antes del build definitivo.

## Criterios de aceptación futuros

- Cero apariciones del selector nativo en al menos 1.000 sesiones o 10.000 selecciones internas.
- El menú SAP no debe permanecer abierto ni activarse al cambiar rápidamente.
- Ningún modificador debe quedar atascado; escribir y usar shortcuts debe funcionar inmediatamente después.
- CPU en reposo: `0.00 %` en una muestra de 20 segundos.
- Memoria privada caliente: no debe crecer linealmente con el número de sesiones.
- Handles, GDI y USER deben estabilizarse después de completar los trabajos de iconos.
- La apertura y la selección no deben empeorar más del 10 % frente a una línea base tomada en el mismo conjunto de ventanas.

## Estado de decisión

- No se recomienda revertir el ajuste de geometría de Eclipse.
- No se recomienda modificar todavía el hook sin capturar evidencia adicional.
- La primera optimización de recursos a evaluar es la poda de la caché de procesos por PID realmente candidatos.
- La primera intervención sobre el bug debe ser instrumentación temporal, no una corrección especulativa.

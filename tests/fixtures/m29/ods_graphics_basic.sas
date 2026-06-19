title 'ODS GRAPHICS: activation et configuration';

/* Test 1 : activer ODS GRAPHICS (sans generation reelle en v1) */
ods graphics on;
ods graphics on / width=1000 height=700;
ods graphics off;

/* Test 2 : log doit confirmer l'activation/desactivation */
ods graphics on;
ods graphics off;

title;

// File input: $fgetc/$ungetc/$fgets/$feof/$ftell/$fseek/$rewind/$fscanf/$sscanf
module top;
  integer fd, r, c, a, b, n;
  reg [8*20:1] line;
  reg [15:0] h;
  reg [7:0] bb;
  reg [8*8:1] s1, s2;
  real x;
  initial begin
    fd = $fopen("fileread.txt", "w");
    $fdisplay(fd, "12 0x3f -7");
    $fdisplay(fd, "hello world");
    $fdisplay(fd, "ab1z 10x1 2.5");
    $fclose(fd);

    fd = $fopen("fileread.txt", "r");
    c = $fgetc(fd);
    $display("getc %0d %c tell %0d", c, c, $ftell(fd));
    r = $ungetc(c, fd);
    r = $fscanf(fd, "%d 0x%h %d", a, h, b);
    $display("fscanf %0d: %0d %h %0d", r, a, h, b);
    r = $fgets(line, fd);
    $display("fgets %0d: '%0s'", r, line);
    r = $fgets(line, fd);
    $display("fgets %0d: '%0s'", r, line);
    r = $fscanf(fd, "%h %b %f", h, bb, x);
    $display("fscanf %0d: %h %b %f", r, h, bb, x);
    r = $fscanf(fd, "%d", a);
    $display("at end: %0d feof %0d", r, $feof(fd) != 0);
    r = $rewind(fd);
    r = $fgets(line, fd);
    $display("rewound: '%0s'", line);
    r = $fseek(fd, 3, 0);
    c = $fgetc(fd);
    $display("seek: %c", c);
    $fclose(fd);

    r = $sscanf("123 hex beaf bin 01xz", "%d hex %h bin %b", a, h, bb);
    $display("sscanf %0d: %0d %h %b", r, a, h, bb);
    r = $sscanf("GOOD alpha  beta", "GOOD %s %s", s1, s2);
    $display("sscanf %0d: %0s %0s", r, s1, s2);
    r = $sscanf("5 6 7", "%d %*d %d", a, b);
    $display("sscanf %0d: %0d %0d", r, a, b);
    r = $sscanf("42abc", "%d%s", a, s1);
    $display("sscanf %0d: %0d %0s", r, a, s1);
    r = $sscanf("x", "%d", a);
    $display("sscanf %0d: %0d", r, a);
    r = $sscanf("", "%d", a);
    $display("sscanf empty %0d", r);
    r = $sscanf("abc", "%d", a);
    $display("sscanf mismatch %0d", r);
    r = $sscanf("1234567", "%3d%d", a, b);
    $display("sscanf width %0d: %0d %0d", r, a, b);
    r = $sscanf("-3.25e1", "%f", x);
    $display("real %f", x);
    r = $sscanf("a b", "%c %c", bb, h);
    $display("chars %0d: %c %c", r, bb, h[7:0]);
  end
endmodule

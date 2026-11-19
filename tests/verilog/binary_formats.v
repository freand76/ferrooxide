// %u and %z: written by $sformat/$fwrite as 32-bit binary words (x/z as 0 for %u, value
// and unknown words for %z) and read back by $sscanf/$fscanf, also with a field width and
// short input; %c and %s of bytes above 127 print as those bytes.
module top;
  reg [63:0] str;
  reg [127:0] wide;
  reg [31:0] in, out;
  reg [47:0] in48, out48;
  integer res, fd;
  initial begin
    in = 32'b000x100z_001z000x_101xxxzz_100z111x;
    $sformat(str, "%u", in);
    res = $sscanf(str, "%u", out);
    $display("u: %0d %b", res, out);
    $sformat(wide, "%z", in);
    res = $sscanf(wide, "%z", out);
    $display("z: %0d %b", res, out);
    in48 = 48'h41_42_43_44_45_46;
    $sformat(wide, "%u", in48);
    res = $sscanf(wide, "%u", out48);
    $display("u48: %0d %h", res, out48);
    res = $sscanf(wide, "%16u", out);
    $display("u16: %0d %h", res, out);
    fd = $fopen("fx_binary_formats.bin", "wb");
    $fwrite(fd, "%u%z", in48, in);
    $fwrite(fd, "%c%c", 8'h01, 8'h02);
    $fclose(fd);
    fd = $fopen("fx_binary_formats.bin", "rb");
    res = $fscanf(fd, "%u%z", out48, out);
    $display("f: %0d %h %b", res, out48, out);
    res = $fscanf(fd, "%u", out);
    $display("short: %0d", res);
    $fclose(fd);
    $display("high: [%c] [%s]", 8'hE9, 16'hC3A9);
  end
endmodule

// expected
// Unpacked array members of unpacked structs (IEEE 1800-2017 7.2): element access with
// constant and run-time indexes, patterns for them (positional, nested, replicated), arrays of
// such structs with two unpacked dimensions, declared in a module and in a block.
module t;
  typedef struct { int a, b[4]; } ab_t;
  typedef struct { logic [3:0] n[2:0]; bit f[2]; } cd_t;
  ab_t s;
  cd_t c;
  int a = 1, b = 2, cc = 3;
  initial begin
    ab_t v1[1:0][2:0];
    int m[2][3];
    s = '{9, '{0, 1, 2, 3}};
    s.b[2] = 7;
    for (int i = 0; i < 4; i++) s.b[i] += 10;
    $display("%0d %0d %0d %0d %0d", s.a, s.b[0], s.b[1], s.b[2], s.b[3]);
    c = '{'{4'h1, 4'h2, 4'h3}, '{1, 0}};
    $display("%h %h %h %b %b", c.n[2], c.n[1], c.n[0], c.f[0], c.f[1]);
    v1 = '{2{'{3{'{a, '{2{b, cc}}}}}}};
    $display("%0d %0d %0d %0d", v1[1][2].a, v1[0][1].b[0], v1[0][1].b[1], v1[1][0].b[3]);
    foreach (m[i, j]) m[i][j] = 10 * i + j;
    $display("%0d %0d %0d", m[0][2], m[1][0], m[1][2]);
  end
endmodule
